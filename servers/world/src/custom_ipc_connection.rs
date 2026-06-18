use crate::{CharaMake, GameData, RemakeMode, WorldDatabase, housing::admin, inventory::Inventory};
use kawari::{
    common::determine_initial_starting_zone,
    config::get_config,
    ipc::kawari::{
        CustomIpcData, CustomIpcSegment, HOUSING_EXPORTS_DIR, clamp_housing_detail_json_for_ipc,
        clamp_housing_export_path_for_ipc, clamp_housing_message_for_ipc,
        clamp_housing_summary_json_for_ipc, validate_housing_import_path_for_ipc,
    },
    packet::{
        CompressionType, ConnectionState, ConnectionType, PacketSegment, SegmentData, SegmentType,
        parse_packet, send_packet,
    },
};

use std::{fs, path::PathBuf, sync::Arc};

use parking_lot::Mutex;
use tokio::net::TcpStream;

/// Represents a single connection between an instance of the world server and the lobby server.
// TODO: Implement the ToServer protocol for CustomIpcConnection so we can notify the global server about some things that happen outside of the world server (e.g., deleted characters need to tell the global state that they're not in those linkshells anymore so online players can be told, and so that a new leader can be promoted if needed)
pub struct CustomIpcConnection {
    pub socket: TcpStream,
    pub state: ConnectionState,
    pub database: Arc<Mutex<WorldDatabase>>,
    pub gamedata: Arc<Mutex<GameData>>,
}

fn housing_summary_response(json: String) -> CustomIpcData {
    CustomIpcData::HousingSummaryResponse {
        json: clamp_housing_summary_json_for_ipc(&json),
    }
}

fn housing_detail_response(json: String) -> CustomIpcData {
    CustomIpcData::HousingEstateDetailResponse {
        json: clamp_housing_detail_json_for_ipc(&json),
    }
}

fn housing_detail_json_for_admin_result(
    land_ident: i64,
    detail_json: Result<Option<String>, serde_json::Error>,
) -> String {
    match detail_json {
        Ok(Some(json)) => json,
        Ok(None) => format!(
            r#"{{"error":"Housing estate {} was not found."}}"#,
            land_ident
        ),
        Err(err) => {
            tracing::warn!(
                "Failed to serialize housing estate detail {land_ident} for admin IPC: {err}"
            );
            format!(
                r#"{{"error":"housing_detail_backend_error","land_ident":{},"message":"Failed to serialize housing estate detail for admin."}}"#,
                land_ident
            )
        }
    }
}

fn housing_mutation_result(message: String) -> CustomIpcData {
    CustomIpcData::HousingEstateMutationResult {
        message: clamp_housing_message_for_ipc(&message),
    }
}

fn housing_exported(path: String, message: String) -> CustomIpcData {
    CustomIpcData::HousingEstateExported {
        path: clamp_housing_export_path_for_ipc(&path),
        message: clamp_housing_message_for_ipc(&message),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn housing_detail_json_for_admin_result_returns_not_found_message() {
        assert_eq!(
            super::housing_detail_json_for_admin_result(42, Ok(None)),
            r#"{"error":"Housing estate 42 was not found."}"#
        );
    }

    #[test]
    fn housing_detail_json_for_admin_result_returns_backend_error_payload() {
        let err =
            serde_json::from_str::<serde_json::Value>("{").expect_err("invalid JSON should fail");
        let payload = super::housing_detail_json_for_admin_result(42, Err(err));
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("backend error payload should be valid JSON");

        assert_eq!(
            parsed["error"].as_str(),
            Some("housing_detail_backend_error")
        );
        assert_eq!(parsed["land_ident"].as_i64(), Some(42));
        assert_eq!(
            parsed["message"].as_str(),
            Some("Failed to serialize housing estate detail for admin.")
        );
    }
}

impl CustomIpcConnection {
    pub fn parse_packet(&mut self, data: &[u8]) -> Vec<PacketSegment<CustomIpcSegment>> {
        parse_packet(data, &mut self.state)
    }

    pub async fn send_custom_response(&mut self, segment: PacketSegment<CustomIpcSegment>) {
        send_packet(
            &mut self.socket,
            &mut self.state,
            ConnectionType::KawariIpc,
            CompressionType::Uncompressed,
            &[segment],
        )
        .await;
    }

    pub async fn handle_custom_ipc(&mut self, data: &CustomIpcSegment) {
        match &data.data {
            CustomIpcData::RequestCreateCharacter {
                service_account_id,
                name,
                chara_make_json,
            } => {
                tracing::info!("creating character from: {name} {chara_make_json}");

                let chara_make = CharaMake::from_json(chara_make_json);

                let city_state;
                {
                    let mut game_data = self.gamedata.lock();

                    city_state = game_data
                        .get_citystate(chara_make.classjob_id as u16)
                        .expect("Unknown citystate");
                }

                let mut inventory = Inventory::default();
                let (content_id, actor_id);
                {
                    let mut game_data = self.gamedata.lock();

                    inventory.equip_classjob_items(chara_make.classjob_id as u16, &mut game_data);

                    // fill inventory
                    inventory.equip_racial_items(
                        chara_make.customize.race,
                        chara_make.customize.gender,
                        &mut game_data,
                    );

                    let mut database = self.database.lock();
                    (content_id, actor_id) = database.create_player_data(
                        *service_account_id,
                        name,
                        chara_make_json,
                        city_state,
                        determine_initial_starting_zone(city_state),
                        inventory,
                        &mut game_data,
                    );
                }

                tracing::info!("Created new player: {content_id} {actor_id}");

                // send them the new actor and content id
                {
                    self.send_custom_response(PacketSegment {
                        segment_type: SegmentType::KawariIpc,
                        data: SegmentData::KawariIpc(CustomIpcSegment::new(
                            CustomIpcData::CharacterCreated {
                                actor_id,
                                content_id,
                            },
                        )),
                        ..Default::default()
                    })
                    .await;
                }
            }
            CustomIpcData::GetActorId { content_id } => {
                let actor_id;
                {
                    let mut database = self.database.lock();
                    actor_id = database.find_actor_id(*content_id);
                }

                tracing::info!("We found an actor id: {actor_id}");

                // send them the actor id
                {
                    self.send_custom_response(PacketSegment {
                        segment_type: SegmentType::KawariIpc,
                        data: SegmentData::KawariIpc(CustomIpcSegment::new(
                            CustomIpcData::ActorIdFound { actor_id },
                        )),
                        ..Default::default()
                    })
                    .await;
                }
            }
            CustomIpcData::CheckNameIsAvailable { name } => {
                let is_name_free;
                {
                    let mut database = self.database.lock();
                    is_name_free = database.check_is_name_free(name);
                }

                // send response
                {
                    self.send_custom_response(PacketSegment {
                        segment_type: SegmentType::KawariIpc,
                        data: SegmentData::KawariIpc(CustomIpcSegment::new(
                            CustomIpcData::NameIsAvailableResponse { free: is_name_free },
                        )),
                        ..Default::default()
                    })
                    .await;
                }
            }
            CustomIpcData::RequestCharacterList { service_account_id } => {
                let config = get_config();

                let characters;
                {
                    let mut game_data = self.gamedata.lock();

                    let mut database = self.database.lock();
                    characters = database.get_character_list(
                        *service_account_id,
                        config.world.world_id,
                        &mut game_data,
                    );
                }

                // send response
                {
                    self.send_custom_response(PacketSegment {
                        segment_type: SegmentType::KawariIpc,
                        data: SegmentData::KawariIpc(CustomIpcSegment::new(
                            CustomIpcData::RequestCharacterListResponse { characters },
                        )),
                        ..Default::default()
                    })
                    .await;
                }
            }
            CustomIpcData::DeleteCharacter { content_id } => {
                {
                    let mut database = self.database.lock();
                    database.delete_character(*content_id);
                }

                // send response
                {
                    self.send_custom_response(PacketSegment {
                        segment_type: SegmentType::KawariIpc,
                        data: SegmentData::KawariIpc(CustomIpcSegment::new(
                            CustomIpcData::CharacterDeleted { deleted: 1 },
                        )),
                        ..Default::default()
                    })
                    .await;
                }
            }
            CustomIpcData::ImportCharacter {
                service_account_id,
                path,
            } => {
                let message;
                {
                    let mut game_data = self.gamedata.lock();
                    let mut database = self.database.lock();
                    if let Err(err) =
                        database.import_character(&mut game_data, *service_account_id, path)
                    {
                        message = err.to_string();
                    } else {
                        message = "Successfully imported!".to_string();
                    }
                }

                // send response
                {
                    self.send_custom_response(PacketSegment {
                        segment_type: SegmentType::KawariIpc,
                        data: SegmentData::KawariIpc(CustomIpcSegment::new(
                            CustomIpcData::CharacterImported { message },
                        )),
                        ..Default::default()
                    })
                    .await;
                }
            }
            CustomIpcData::RemakeCharacter {
                content_id,
                chara_make_json,
            } => {
                {
                    let mut database = self.database.lock();

                    // overwrite it in the database
                    database.set_chara_make(*content_id, chara_make_json);

                    // reset flag
                    database.set_remake_mode(*content_id, RemakeMode::None);
                }

                // send response
                {
                    self.send_custom_response(PacketSegment {
                        segment_type: SegmentType::KawariIpc,
                        data: SegmentData::KawariIpc(CustomIpcSegment::new(
                            CustomIpcData::CharacterRemade {
                                content_id: *content_id,
                            },
                        )),
                        ..Default::default()
                    })
                    .await;
                }
            }
            CustomIpcData::DeleteServiceAccount { service_account_id } => {
                let mut database = self.database.lock();
                database.delete_characters(*service_account_id);
            }
            CustomIpcData::RequestFullCharacterList {} => {
                let json;
                {
                    let mut database = self.database.lock();
                    json = database.request_full_character_list();
                }

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(
                        CustomIpcData::FullCharacterListResponse { json },
                    )),
                    ..Default::default()
                })
                .await;
            }
            CustomIpcData::RequestHousingSummary {} => {
                let json = {
                    let mut database = self.database.lock();
                    let rows = database.housing_summary_rows_for_admin();
                    admin::summary_json(&rows)
                };

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(housing_summary_response(
                        json,
                    ))),
                    ..Default::default()
                })
                .await;
            }
            CustomIpcData::RequestHousingEstateDetail { land_ident } => {
                let json = {
                    let mut database = self.database.lock();
                    let detail_json = database
                        .housing_estate_detail_for_admin(*land_ident)
                        .map(|detail| admin::detail_ipc_json(&detail))
                        .transpose();
                    housing_detail_json_for_admin_result(*land_ident, detail_json)
                };

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(housing_detail_response(
                        json,
                    ))),
                    ..Default::default()
                })
                .await;
            }
            CustomIpcData::ResetHousingFurniture { land_ident } => {
                let message = {
                    let mut database = self.database.lock();
                    if database
                        .housing_estate_detail_for_admin(*land_ident)
                        .is_none()
                    {
                        format!("Housing estate {} was not found.", land_ident)
                    } else {
                        let deleted = database.delete_housing_furniture_for_estate(*land_ident);
                        format!("Deleted {deleted} furniture rows for estate {land_ident}.")
                    }
                };

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(housing_mutation_result(
                        message,
                    ))),
                    ..Default::default()
                })
                .await;
            }
            CustomIpcData::ResetHousingEstate { land_ident } => {
                let message = {
                    let mut database = self.database.lock();
                    if database.delete_housing_estate_and_furniture(*land_ident) {
                        format!("Deleted estate {land_ident} and its furniture rows.")
                    } else {
                        format!("Housing estate {} was not found.", land_ident)
                    }
                };

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(housing_mutation_result(
                        message,
                    ))),
                    ..Default::default()
                })
                .await;
            }
            CustomIpcData::UpdateHousingEstateText {
                land_ident,
                name,
                greeting,
            } => {
                let message = {
                    let mut database = self.database.lock();
                    let updated_name = database.update_housing_name(*land_ident, name);
                    let updated_greeting = database.update_housing_greeting(*land_ident, greeting);

                    if updated_name || updated_greeting {
                        format!("Updated estate text for {land_ident}.")
                    } else {
                        format!("Housing estate {} was not found.", land_ident)
                    }
                };

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(housing_mutation_result(
                        message,
                    ))),
                    ..Default::default()
                })
                .await;
            }
            CustomIpcData::ExportHousingEstate { land_ident } => {
                let export_result = {
                    let mut database = self.database.lock();
                    database.export_housing_estate(*land_ident)
                };

                let mut path = String::new();
                let message = match export_result {
                    Some(export) => {
                        let export_dir = PathBuf::from(HOUSING_EXPORTS_DIR);
                        let export_path = export_dir.join(format!("estate-{land_ident}.json"));

                        match fs::create_dir_all(&export_dir) {
                            Ok(()) => match serde_json::to_string_pretty(&export) {
                                Ok(json) => match fs::write(&export_path, json) {
                                    Ok(()) => {
                                        path = export_path.to_string_lossy().into_owned();
                                        format!("Exported estate {land_ident} to {path}.")
                                    }
                                    Err(err) => format!(
                                        "Failed to write export for estate {land_ident}: {err}"
                                    ),
                                },
                                Err(err) => {
                                    format!("Failed to serialize estate {land_ident} export: {err}")
                                }
                            },
                            Err(err) => format!(
                                "Failed to create housing export directory for estate {land_ident}: {err}"
                            ),
                        }
                    }
                    None => format!("Housing estate {} was not found.", land_ident),
                };

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(housing_exported(
                        path, message,
                    ))),
                    ..Default::default()
                })
                .await;
            }
            CustomIpcData::ImportHousingEstate { path } => {
                let message = match validate_housing_import_path_for_ipc(path) {
                    Ok(path) => {
                        let import_path = PathBuf::from(&path);
                        match fs::read_to_string(&import_path) {
                            Ok(contents) => match serde_json::from_str(&contents) {
                                Ok(export) => {
                                    let mut database = self.database.lock();
                                    if database.import_housing_estate(export) {
                                        format!("Imported housing estate from {path}.")
                                    } else {
                                        format!("Failed to import housing estate from {path}.")
                                    }
                                }
                                Err(err) => {
                                    format!("Failed to parse housing import file {path}: {err}")
                                }
                            },
                            Err(err) => {
                                format!("Failed to read housing import file {path}: {err}")
                            }
                        }
                    }
                    Err(message) => message,
                };

                self.send_custom_response(PacketSegment {
                    segment_type: SegmentType::KawariIpc,
                    data: SegmentData::KawariIpc(CustomIpcSegment::new(housing_mutation_result(
                        message,
                    ))),
                    ..Default::default()
                })
                .await;
            }
            _ => {
                panic!("The server is recieving a response or unknown custom IPC! {data:#?}")
            }
        }
    }
}
