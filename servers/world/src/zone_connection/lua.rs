//! Translates tasks and handles other information from `LuaPlayer`.

use std::path::PathBuf;

use super::LastHousingPreset;
use super::housing::{
    update_exterior_json_color, update_exterior_json_field, update_interior_json_field,
    update_interior_json_renovation_row_id,
};
use super::remake_place::{
    RemakePlaceImportRows, RemakePlaceInteriorFixtureUpdates, RemakePlacePresetPath,
    build_remake_place_furniture_rows, build_remake_place_interior_fixture_updates,
    parse_remake_place_layout_file, resolve_latest_remake_place_preset_path,
    resolve_remake_place_preset_path,
};
use crate::housing::apartment::MAX_APARTMENT_ROOM_NUMBER;
use crate::{
    Event, HousingEstate, HousingEstateSpec, ItemInfoQuery, ToServer, WorldDatabase,
    ZoneConnection,
    event::EventHandler,
    inventory::{CrystalsStorage, CurrencyStorage, Item},
    lua::{
        HousingEstateKind, HousingExteriorColorField, HousingExteriorField, HousingInteriorField,
        HousingKit, HousingPresetScope, HousingResetMode, LuaPlayer, LuaTask,
    },
};
use kawari::{
    common::{
        ContainerType, DirectorEvent, ERR_INVENTORY_ADD_FAILED, HandlerId, InstanceContentType,
        ObjectTypeId, ObjectTypeKind,
    },
    constants::{
        ADVENTURE_BITMASK_SIZE, AETHER_CURRENT_BITMASK_SIZE,
        AETHER_CURRENT_COMP_FLG_SET_BITMASK_SIZE, BUDDY_EQUIP_BITMASK_SIZE,
        CAUGHT_FISH_BITMASK_SIZE, CAUGHT_SPEARFISH_BITMASK_SIZE, CHOCOBO_TAXI_STANDS_BITMASK_SIZE,
        CUTSCENE_SEEN_BITMASK_SIZE, GLASSES_STYLES_BITMASK_SIZE, MINION_BITMASK_SIZE,
        ORNAMENT_BITMASK_SIZE, TRIPLE_TRIAD_CARDS_BITMASK_SIZE,
    },
    ipc::zone::{
        ActorControlCategory, ActorControlSelf, ClientTriggerCommand, ItemInfo, PlotSize,
        ServerZoneIpcData, ServerZoneIpcSegment,
    },
};

impl ZoneConnection {
    pub async fn process_lua_player(
        &mut self,
        player: &mut LuaPlayer,
        events: &mut Vec<(Box<dyn EventHandler>, Event)>,
    ) -> bool {
        // First, send zone-related segments
        for segment in &player.zone_data.queued_segments {
            let mut edited_segment = segment.clone();
            edited_segment.target_actor = player.player_data.character.actor_id;
            self.send_segment(edited_segment).await;
        }
        player.zone_data.queued_segments.clear();

        // These are to run functions that could possibly generate more tasks.
        // We can't do this in the loop!'
        let mut run_finish_event = false;
        let mut continue_nesting = false;

        let tasks = player.queued_tasks.clone();
        player.queued_tasks.clear();
        for task in &tasks {
            match task {
                LuaTask::ChangeTerritory {
                    zone_id,
                    exit_position,
                    exit_rotation,
                } => {
                    self.change_zone(*zone_id, *exit_position, *exit_rotation, None)
                        .await
                }
                LuaTask::SetRemakeMode(remake_mode) => {
                    let mut database = self.database.lock();
                    database.set_remake_mode(
                        player.player_data.character.content_id as u64,
                        *remake_mode,
                    );
                }
                LuaTask::Warp { warp_id } => {
                    self.warp(*warp_id).await;
                }
                LuaTask::BeginLogOut => self.begin_log_out().await,
                LuaTask::FinishEvent {} => {
                    self.event_finish(events).await;
                    run_finish_event = true;
                }
                LuaTask::UnlockClassJob { classjob_id } => {
                    let starting_level;
                    let soul_crystal_id;
                    {
                        let mut gamedata = self.gamedata.lock();

                        starting_level = gamedata
                            .get_starting_level(*classjob_id as u16)
                            .unwrap_or(1);
                        soul_crystal_id = gamedata.get_soul_crystal_item_id(*classjob_id as u16);
                    }

                    self.set_level_for(*classjob_id, starting_level as u16);

                    self.actor_control_self(ActorControlCategory::UnlockClass {
                        classjob_id: *classjob_id as u32,
                    })
                    .await;

                    // UnlockClass only sets it to level 1, but we want to change the level.
                    self.actor_control_self(ActorControlCategory::SetLevel {
                        classjob_id: *classjob_id as u32,
                        level: starting_level as u32,
                    })
                    .await;

                    if let Some(item_id) = soul_crystal_id {
                        let soul_crystal = Item {
                            quantity: 1,
                            item_id,
                            ..Default::default()
                        };

                        let destination = self
                            .player_data
                            .inventory
                            .add_in_next_free_armory_slot(13)
                            .unwrap();
                        self.player_data.inventory.add_in_slot(
                            soul_crystal,
                            &destination.container,
                            destination.slot,
                        );
                        self.send_inventory().await;
                    }
                }
                LuaTask::WarpAetheryte {
                    aetheryte_id,
                    housing_aethernet,
                } => {
                    self.warp_aetheryte(*aetheryte_id, *housing_aethernet, false)
                        .await;
                }
                LuaTask::ToggleInvisibility { invisible } => {
                    self.toggle_invisibility(*invisible).await;
                }
                LuaTask::Unlock { id } => {
                    self.player_data.unlock.unlocks.set(*id);

                    self.actor_control_self(ActorControlCategory::ToggleUnlock {
                        id: *id,
                        unlocked: true,
                    })
                    .await;
                }
                LuaTask::UnlockAll {} => {
                    self.player_data.unlock.unlocks.set_all();
                }
                LuaTask::UnlockAetheryte { id, on } => {
                    let unlock_all = *id == 0;
                    if unlock_all {
                        for i in 1..239 {
                            if *on {
                                self.player_data.aetheryte.unlocked.set(i);
                            } else {
                                self.player_data.aetheryte.unlocked.clear(i);
                            }

                            self.actor_control_self(ActorControlCategory::LearnTeleport {
                                id: i,
                                unlocked: *on,
                            })
                            .await;
                        }
                    } else {
                        if *on {
                            self.player_data.aetheryte.unlocked.set(*id);
                        } else {
                            self.player_data.aetheryte.unlocked.clear(*id);
                        }

                        self.actor_control_self(ActorControlCategory::LearnTeleport {
                            id: *id,
                            unlocked: *on,
                        })
                        .await;
                    }
                }
                LuaTask::SetLevel { level } => {
                    self.set_current_level(*level);
                    self.update_class_info().await;
                    self.send_stats().await; // Needed because stats change based on level.
                }
                LuaTask::ChangeWeather { id } => {
                    self.change_weather(*id).await;
                }
                LuaTask::ModifyCurrency {
                    id,
                    amount,
                    send_client_update,
                } => {
                    let slot = self.player_data.inventory.currency.get_item_for_id(*id);

                    if *amount > 0 {
                        slot.quantity = slot.quantity.saturating_add(*amount as u32);
                    } else {
                        slot.quantity = slot.quantity.saturating_sub(-(*amount) as u32);
                    }

                    if *send_client_update {
                        let slot = *slot;

                        let ipc = ServerZoneIpcSegment::new(
                            ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
                                sequence: self.player_data.item_sequence,
                                container: ContainerType::Currency,
                                slot: CurrencyStorage::get_slot_for_id(*id),
                                ..slot.into()
                            }),
                        );
                        self.send_ipc_self(ipc).await;
                    }

                    {
                        let mut database = self.database.lock();
                        database.commit_classjob_and_inventory(&self.player_data);
                    }
                }
                LuaTask::ModifyCrystal {
                    id,
                    amount,
                    send_client_update,
                } => {
                    let slot = self.player_data.inventory.crystals.get_item_for_id(*id);

                    if *amount > 0 {
                        slot.quantity = slot.quantity.saturating_add(*amount as u32);
                    } else {
                        slot.quantity = slot.quantity.saturating_sub(-(*amount) as u32);
                    }

                    if *send_client_update {
                        let slot = *slot;

                        let ipc = ServerZoneIpcSegment::new(
                            ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
                                sequence: self.player_data.item_sequence,
                                container: ContainerType::Currency,
                                slot: CrystalsStorage::get_slot_for_id(*id),
                                ..slot.into()
                            }),
                        );
                        self.send_ipc_self(ipc).await;
                    }

                    {
                        let mut database = self.database.lock();
                        database.commit_classjob_and_inventory(&self.player_data);
                    }
                }
                LuaTask::GmSetOrchestrion { value, id } => {
                    self.gm_set_orchestrion(*value, *id);
                }
                LuaTask::ToggleOrchestrion { id } => {
                    self.toggle_orchestrion(*id).await;
                }
                LuaTask::AddItem {
                    id,
                    quantity,
                    send_client_update,
                } => {
                    let new_item: Option<Item>;
                    {
                        let mut game_data = self.gamedata.lock();
                        new_item = game_data
                            .get_item_info(ItemInfoQuery::ById(*id))
                            .map(|x| Item::new(&x, *quantity));
                    }
                    if let Some(new_item) = new_item {
                        if self
                            .player_data
                            .inventory
                            .add_in_next_free_slot(new_item)
                            .is_some()
                        {
                            {
                                let mut database = self.database.lock();
                                database.commit_classjob_and_inventory(&self.player_data);
                            }

                            if *send_client_update {
                                self.send_inventory().await;
                            }
                        } else {
                            tracing::error!(ERR_INVENTORY_ADD_FAILED);
                            self.send_notice(ERR_INVENTORY_ADD_FAILED).await;
                        }
                    } else {
                        tracing::error!(ERR_INVENTORY_ADD_FAILED);
                        self.send_notice(ERR_INVENTORY_ADD_FAILED).await;
                    }
                }
                LuaTask::ShowHousingPlacard {
                    ward_index,
                    division,
                    plot_index,
                } => {
                    self.send_housing_placard_info(
                        self.player_data.volatile.zone_id as u16,
                        *ward_index,
                        *division,
                        *plot_index,
                    )
                    .await;
                }
                LuaTask::EnsureLocalApartment { room_number } => {
                    let Some(estate) = ({
                        let context = self.apartment_ward_context_or_default();
                        let mut database = self.database.lock();
                        database.ensure_local_apartment(
                            self.player_data.character.content_id as u64,
                            &self.player_data.character.name,
                            self.config.world_id,
                            context.territory_type_id,
                            context.ward_index,
                            context.division,
                            *room_number,
                        )
                    }) else {
                        self.send_notice(&format!(
                            "Apartment room numbers must be between 1 and {MAX_APARTMENT_ROOM_NUMBER}."
                        ))
                        .await;
                        continue;
                    };
                    self.set_active_housing_ward_context_from_estate(&estate);
                    self.send_notice(&format!(
                        "Local apartment ready: room {} house_id=0x{:016X} land_ident={}",
                        room_number, estate.house_id as u64, estate.land_ident
                    ))
                    .await;
                    self.send_owned_housing().await;
                }
                LuaTask::EnsureLocalHouse {} => {
                    let (house_id, land_ident, estate_name) = {
                        let mut database = self.database.lock();
                        let estate = database.ensure_local_estate(
                            self.player_data.character.content_id as u64,
                            &self.player_data.character.name,
                            self.config.world_id,
                        );

                        (
                            estate.house_id as u64,
                            estate.land_ident,
                            estate.estate_name,
                        )
                    };

                    self.send_notice(&format!(
                        "Local housing estate ready: {estate_name} house_id=0x{house_id:016X} land_ident={land_ident}"
                    ))
                    .await;
                    self.send_owned_housing().await;
                }
                LuaTask::EnsureLocalHouseWithOptions {
                    kind,
                    size,
                    territory_type_id,
                    ward_index,
                    division,
                    plot_index,
                } => {
                    let (house_id, land_ident, estate_name, estate) = {
                        let mut database = self.database.lock();
                        let estate = database.ensure_local_estate_with_spec(HousingEstateSpec {
                            owner_content_id: self.player_data.character.content_id as u64,
                            owner_name: self.player_data.character.name.clone(),
                            world_id: self.config.world_id,
                            territory_type_id: *territory_type_id,
                            ward_index: *ward_index,
                            division: *division,
                            plot_index: *plot_index,
                            plot_size: *size,
                            free_company: *kind == HousingEstateKind::FreeCompany,
                        });

                        (
                            estate.house_id as u64,
                            estate.land_ident,
                            estate.estate_name.clone(),
                            estate,
                        )
                    };
                    self.set_active_housing_estate_from_row(&estate, false);

                    self.send_notice(&format!(
                        "Local housing estate ready: {estate_name} house_id=0x{house_id:016X} land_ident={land_ident}"
                    ))
                    .await;
                    self.send_owned_housing().await;
                }
                LuaTask::ResetHousing { mode } => match mode {
                    HousingResetMode::Furniture => {
                        if let Some(estate) = self.current_or_owned_housing_estate() {
                            let deleted = {
                                let mut database = self.database.lock();
                                database.delete_housing_furniture_for_estate(estate.land_ident)
                            };
                            self.clear_housing_furniture_reset_cache();
                            self.send_notice(&format!(
                                "Deleted {deleted} housing furniture rows for {}.",
                                estate.estate_name
                            ))
                            .await;
                        } else {
                            self.send_notice("No local housing estate found to reset.")
                                .await;
                        }
                    }
                    HousingResetMode::Estate => {
                        if let Some(estate) = self.current_or_owned_housing_estate() {
                            let deleted = {
                                let mut database = self.database.lock();
                                database.delete_housing_estate_and_furniture(estate.land_ident)
                            };
                            if deleted {
                                self.clear_housing_estate_reset_cache();
                                self.send_notice(&format!(
                                    "Deleted local housing estate {}.",
                                    estate.estate_name
                                ))
                                .await;
                                self.send_owned_housing().await;
                            } else {
                                self.send_notice("No local housing estate found to reset.")
                                    .await;
                            }
                        } else {
                            self.send_notice("No local housing estate found to reset.")
                                .await;
                        }
                    }
                    HousingResetMode::All => {
                        let deleted = {
                            let mut database = self.database.lock();
                            let estates = database.owned_housing_estates(
                                self.player_data.character.content_id as u64,
                            );
                            let count = estates.len();
                            for estate in estates {
                                database.delete_housing_estate_and_furniture(estate.land_ident);
                            }
                            count
                        };
                        self.clear_housing_estate_reset_cache();
                        self.send_notice(&format!("Deleted {deleted} local housing estates."))
                            .await;
                        self.send_owned_housing().await;
                    }
                },
                LuaTask::UpdateHousingName { name } => {
                    if let Some(estate) = self.current_or_owned_housing_estate() {
                        let updated = {
                            let mut database = self.database.lock();
                            database.update_housing_name(estate.land_ident, name)
                        };
                        if updated {
                            self.send_notice(&format!("Housing estate name set to {name}."))
                                .await;
                        } else {
                            self.send_notice("No local housing estate found to update.")
                                .await;
                        }
                    } else {
                        self.send_notice("No local housing estate found to update.")
                            .await;
                    }
                }
                LuaTask::UpdateHousingGreeting { greeting } => {
                    if let Some(estate) = self.current_or_owned_housing_estate() {
                        let updated = {
                            let mut database = self.database.lock();
                            database.update_housing_greeting(estate.land_ident, greeting)
                        };
                        if updated {
                            self.send_notice("Housing estate greeting updated.").await;
                        } else {
                            self.send_notice("No local housing estate found to update.")
                                .await;
                        }
                    } else {
                        self.send_notice("No local housing estate found to update.")
                            .await;
                    }
                }
                LuaTask::UpdateHousingLight { level } => {
                    if let Some(estate) = self.current_or_owned_housing_estate() {
                        let level = (*level).min(5);
                        let updated = {
                            let mut database = self.database.lock();
                            database.update_housing_light_level(estate.land_ident, level)
                        };
                        if updated {
                            self.send_notice(&format!("Housing light level set to {level}."))
                                .await;
                        } else {
                            self.send_notice("No local housing estate found to update.")
                                .await;
                        }
                    } else {
                        self.send_notice("No local housing estate found to update.")
                            .await;
                    }
                }
                LuaTask::UpdateHousingExterior { field, value } => {
                    if let Some(estate) = self.current_or_owned_housing_estate() {
                        let outcome = {
                            let mut database = self.database.lock();
                            apply_housing_exterior_fixture_update(
                                &mut database,
                                &estate,
                                *field,
                                *value,
                            )
                        };
                        match outcome {
                            HousingFixtureUpdateResult::Updated => {
                                self.send_notice(
                                    "Housing exterior updated. Re-enter the ward to refresh visuals.",
                                )
                                .await;
                            }
                            HousingFixtureUpdateResult::MissingEstate => {
                                self.send_notice("No local housing estate found to update.")
                                    .await;
                            }
                            HousingFixtureUpdateResult::InvalidStoredJson => {
                                self.send_notice(
                                    "Housing exterior update failed because the stored fixture JSON is invalid.",
                                )
                                .await;
                            }
                        }
                    } else {
                        self.send_notice("No local housing estate found to update.")
                            .await;
                    }
                }
                LuaTask::UpdateHousingExteriorColor { field, value } => {
                    if let Some(estate) = self.current_or_owned_housing_estate() {
                        let outcome = {
                            let mut database = self.database.lock();
                            apply_housing_exterior_color_update(
                                &mut database,
                                &estate,
                                *field,
                                *value,
                            )
                        };
                        match outcome {
                            HousingFixtureUpdateResult::Updated => {
                                self.send_notice(
                                    "Housing exterior color updated. Re-enter the ward to refresh visuals.",
                                )
                                .await;
                            }
                            HousingFixtureUpdateResult::MissingEstate => {
                                self.send_notice("No local housing estate found to update.")
                                    .await;
                            }
                            HousingFixtureUpdateResult::InvalidStoredJson => {
                                self.send_notice(
                                    "Housing exterior color update failed because the stored fixture JSON is invalid.",
                                )
                                .await;
                            }
                        }
                    } else {
                        self.send_notice("No local housing estate found to update.")
                            .await;
                    }
                }
                LuaTask::UpdateHousingInterior { field, value } => {
                    if let Some(estate) = self.current_or_owned_housing_estate() {
                        let outcome = {
                            let mut database = self.database.lock();
                            apply_housing_interior_fixture_update(
                                &mut database,
                                &estate,
                                *field,
                                *value,
                            )
                        };
                        match outcome {
                            HousingFixtureUpdateResult::Updated => {
                                self.send_notice(
                                    "Housing interior updated. Re-enter the estate to refresh fixtures.",
                                )
                                .await;
                            }
                            HousingFixtureUpdateResult::MissingEstate => {
                                self.send_notice("No local housing estate found to update.")
                                    .await;
                            }
                            HousingFixtureUpdateResult::InvalidStoredJson => {
                                self.send_notice(
                                    "Housing interior update failed because the stored fixture JSON is invalid.",
                                )
                                .await;
                            }
                        }
                    } else {
                        self.send_notice("No local housing estate found to update.")
                            .await;
                    }
                }
                LuaTask::ApplyHousingPreset {
                    path,
                    scope,
                    reload,
                } => {
                    let Some(estate) = self.current_or_owned_housing_estate() else {
                        self.send_notice("No local housing estate found to apply the preset.")
                            .await;
                        continue;
                    };

                    match resolve_remake_place_preset_path(path).and_then(|preset_path| {
                        apply_remake_place_preset_to_estate(
                            self,
                            &estate,
                            preset_path,
                            *scope,
                            *reload,
                        )
                    }) {
                        Ok(outcome) => {
                            self.last_housing_preset = Some(LastHousingPreset {
                                path: outcome.path,
                                scope: *scope,
                            });
                            tracing::debug!(summary = %outcome.summary, "Applied ReMakePlace housing preset");
                            self.send_notice(&outcome.notice).await;
                            if *reload {
                                reload_housing_after_preset(self, *scope).await;
                            }
                        }
                        Err(error) => self.send_notice(&error).await,
                    }
                }
                LuaTask::ApplyLatestHousingPreset { scope, reload } => {
                    let Some(estate) = self.current_or_owned_housing_estate() else {
                        self.send_notice("No local housing estate found to apply the preset.")
                            .await;
                        continue;
                    };

                    match resolve_latest_remake_place_preset_path().and_then(|preset_path| {
                        apply_remake_place_preset_to_estate(
                            self,
                            &estate,
                            preset_path,
                            *scope,
                            *reload,
                        )
                    }) {
                        Ok(outcome) => {
                            self.last_housing_preset = Some(LastHousingPreset {
                                path: outcome.path,
                                scope: *scope,
                            });
                            tracing::debug!(summary = %outcome.summary, "Applied latest ReMakePlace housing preset");
                            self.send_notice(&outcome.notice).await;
                            if *reload {
                                reload_housing_after_preset(self, *scope).await;
                            }
                        }
                        Err(error) => self.send_notice(&error).await,
                    }
                }
                LuaTask::RepeatHousingPreset { reload } => {
                    let Some(estate) = self.current_or_owned_housing_estate() else {
                        self.send_notice("No local housing estate found to apply the preset.")
                            .await;
                        continue;
                    };
                    let Some(last_preset) = self.last_housing_preset.clone() else {
                        self.send_notice(
                            "No successful ReMakePlace preset has been applied in this session.",
                        )
                        .await;
                        continue;
                    };

                    let scope = last_preset.scope;
                    match resolve_remake_place_preset_path(&last_preset.path.to_string_lossy())
                        .and_then(|preset_path| {
                            apply_remake_place_preset_to_estate(
                                self,
                                &estate,
                                preset_path,
                                scope,
                                *reload,
                            )
                        }) {
                        Ok(outcome) => {
                            self.last_housing_preset = Some(LastHousingPreset {
                                path: outcome.path,
                                scope,
                            });
                            tracing::debug!(summary = %outcome.summary, "Repeated ReMakePlace housing preset");
                            self.send_notice(&outcome.notice).await;
                            if *reload {
                                reload_housing_after_preset(self, scope).await;
                            }
                        }
                        Err(error) => self.send_notice(&error).await,
                    }
                }
                LuaTask::CheckHousingPreset { path, scope } => {
                    tracing::debug!(
                        path = %path,
                        scope = housing_preset_scope_label(*scope),
                        "Checking ReMakePlace housing preset from explicit path"
                    );
                    let Some(estate) = self.current_or_owned_housing_estate() else {
                        self.send_notice("No local housing estate found to check the preset.")
                            .await;
                        continue;
                    };

                    match resolve_remake_place_preset_path(path).and_then(|preset_path| {
                        check_remake_place_preset_for_estate(self, &estate, preset_path, *scope)
                    }) {
                        Ok(summary) => self.send_notice(&summary).await,
                        Err(error) => self.send_notice(&error).await,
                    }
                }
                LuaTask::CheckLatestHousingPreset { scope } => {
                    tracing::debug!(
                        scope = housing_preset_scope_label(*scope),
                        "Checking latest ReMakePlace housing preset"
                    );
                    let Some(estate) = self.current_or_owned_housing_estate() else {
                        self.send_notice("No local housing estate found to check the preset.")
                            .await;
                        continue;
                    };

                    match resolve_latest_remake_place_preset_path().and_then(|preset_path| {
                        check_remake_place_preset_for_estate(self, &estate, preset_path, *scope)
                    }) {
                        Ok(summary) => self.send_notice(&summary).await,
                        Err(error) => self.send_notice(&error).await,
                    }
                }
                LuaTask::CheckRepeatedHousingPreset {} => {
                    tracing::debug!("Checking repeated ReMakePlace housing preset");
                    let Some(estate) = self.current_or_owned_housing_estate() else {
                        self.send_notice("No local housing estate found to check the preset.")
                            .await;
                        continue;
                    };
                    let Some(last_preset) = self.last_housing_preset.clone() else {
                        self.send_notice(
                            "No successful ReMakePlace preset has been applied in this session.",
                        )
                        .await;
                        continue;
                    };

                    match resolve_remake_place_preset_path(&last_preset.path.to_string_lossy())
                        .and_then(|preset_path| {
                            check_remake_place_preset_for_estate(
                                self,
                                &estate,
                                preset_path,
                                last_preset.scope,
                            )
                        }) {
                        Ok(summary) => self.send_notice(&summary).await,
                        Err(error) => self.send_notice(&error).await,
                    }
                }
                LuaTask::GiveHousingKit { kit } => {
                    let mut added = 0;
                    let mut failed = 0;

                    for item_id in housing_kit_items(*kit) {
                        let new_item = {
                            let mut game_data = self.gamedata.lock();
                            game_data
                                .get_item_info(ItemInfoQuery::ById(*item_id))
                                .map(|info| Item::new(&info, 1))
                        };

                        if let Some(new_item) = new_item
                            && self
                                .player_data
                                .inventory
                                .add_in_next_free_slot(new_item)
                                .is_some()
                        {
                            added += 1;
                        } else {
                            failed += 1;
                        }
                    }

                    if added > 0 {
                        {
                            let mut database = self.database.lock();
                            database.commit_classjob_and_inventory(&self.player_data);
                        }
                        self.send_inventory().await;
                    }

                    if failed == 0 {
                        self.send_notice(&format!("Added {added} housing kit items."))
                            .await;
                    } else {
                        self.send_notice(&format!(
                            "Added {added} housing kit items; {failed} failed."
                        ))
                        .await;
                    }
                }
                LuaTask::EnterLocalApartment { room_number } => {
                    self.enter_local_apartment(*room_number).await;
                }
                LuaTask::EnterLocalHouse {} => {
                    self.enter_local_house().await;
                }
                LuaTask::ExitLocalHouse {} => {
                    self.exit_local_house().await;
                }
                LuaTask::ReloadHousing {} => {
                    self.reload_current_housing_interior().await;
                }
                LuaTask::UnlockContent { id } => {
                    {
                        let mut game_data = self.gamedata.lock();
                        if let Some(instance_content_type) = game_data.find_type_for_content(*id) {
                            // Each id has to be subtracted by it's offset in the InstanceContent Excel sheet. For example, all guildheists start at ID 10000.
                            match instance_content_type {
                                InstanceContentType::Dungeon => {
                                    self.player_data
                                        .content
                                        .unlocked_dungeons
                                        .set(*id as u32 - 1);
                                }
                                InstanceContentType::Raid => {
                                    self.player_data
                                        .content
                                        .unlocked_raids
                                        .set(*id as u32 - 30001);
                                }
                                InstanceContentType::Guildhests => {
                                    self.player_data
                                        .content
                                        .unlocked_guildhests
                                        .set(*id as u32 - 10001);
                                }
                                InstanceContentType::Trial => {
                                    self.player_data
                                        .content
                                        .unlocked_trials
                                        .set(*id as u32 - 20001);
                                }
                                _ => {
                                    tracing::warn!(
                                        "Not sure what to do about {instance_content_type:?} {id}!"
                                    );
                                }
                            };
                        } else {
                            tracing::warn!("Unknown content {id}!");
                        }
                    }

                    self.actor_control_self(ActorControlCategory::UnlockInstanceContent {
                        id: *id as u32,
                        unlocked: true,
                    })
                    .await;
                }
                LuaTask::UnlockAllContent {} => {
                    self.player_data.content.unlocked_special_content.set_all();
                    self.player_data.content.unlocked_raids.set_all();
                    self.player_data.content.unlocked_dungeons.set_all();
                    self.player_data.content.unlocked_guildhests.set_all();
                    self.player_data.content.unlocked_trials.set_all();
                    self.player_data
                        .content
                        .unlocked_crystalline_conflicts
                        .set_all();
                    self.player_data.content.unlocked_frontlines.set_all();
                    self.player_data.content.unlocked_misc_content.set_all();
                }
                LuaTask::AddExp { amount } => {
                    self.add_exp(*amount).await;
                }
                LuaTask::StartEvent {
                    event_id,
                    event_type,
                    event_arg,
                } => {
                    let target_object;
                    if let Some(event) = events.last() {
                        target_object = event.1.actor_id;
                    } else {
                        // Fall back to the player as a sensible default
                        target_object = ObjectTypeId {
                            object_id: self.player_data.character.actor_id,
                            object_type: ObjectTypeKind::None,
                        };
                    }

                    self.start_event(
                        target_object,
                        *event_id,
                        *event_type,
                        *event_arg,
                        events,
                        player,
                    )
                    .await;
                }
                LuaTask::SetInnWakeup { watched } => {
                    self.player_data.saw_inn_wakeup = *watched;
                }
                LuaTask::ToggleMount { id } => {
                    let order;
                    {
                        let mut game_data = self.gamedata.lock();
                        order = game_data.find_mount_order(*id).unwrap_or(0);
                    }

                    let should_unlock = self.player_data.unlock.mounts.toggle(order as u32);

                    self.actor_control_self(ActorControlCategory::ToggleMountUnlock {
                        order: order as u32,
                        id: *id,
                        unlocked: should_unlock,
                    })
                    .await;
                }
                LuaTask::MoveToPopRange { id, fade_out } => {
                    self.handle
                        .send(ToServer::MoveToPopRange(
                            self.id,
                            self.player_data.character.actor_id,
                            *id,
                            *fade_out,
                        ))
                        .await;
                }
                LuaTask::SetHP { hp } => {
                    self.handle
                        .send(ToServer::SetHP(
                            self.id,
                            self.player_data.character.actor_id,
                            *hp,
                        ))
                        .await;
                }
                LuaTask::SetMP { mp } => {
                    self.handle
                        .send(ToServer::SetMP(
                            self.id,
                            self.player_data.character.actor_id,
                            *mp,
                        ))
                        .await;
                }
                LuaTask::ToggleGlassesStyle { id } => {
                    self.toggle_glasses_style(*id).await;
                }
                LuaTask::ToggleGlassesStyleAll {} => {
                    let max_glasses_style_id = GLASSES_STYLES_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_glasses_style_id {
                        self.toggle_glasses_style(i).await;
                    }
                }
                LuaTask::ToggleOrnament { id } => {
                    self.toggle_ornament(*id).await;
                }
                LuaTask::ToggleOrnamentAll {} => {
                    let max_ornament_id = ORNAMENT_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_ornament_id {
                        self.toggle_ornament(i).await;
                    }
                }
                LuaTask::UnlockBuddyEquip { id } => {
                    self.unlock_buddy_equip(*id).await;
                }
                LuaTask::UnlockBuddyEquipAll {} => {
                    let max_buddy_equip_id = BUDDY_EQUIP_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_buddy_equip_id {
                        self.unlock_buddy_equip(i).await;
                    }
                }
                LuaTask::ToggleChocoboTaxiStand { id } => {
                    self.toggle_chocobo_taxi_stand(*id).await;
                }
                LuaTask::ToggleChocoboTaxiStandAll {} => {
                    let max_chocobo_taxi_stand_id = CHOCOBO_TAXI_STANDS_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_chocobo_taxi_stand_id {
                        self.toggle_chocobo_taxi_stand(i).await;
                    }
                }
                LuaTask::ToggleCaughtFish { id } => {
                    self.toggle_caught_fish(*id).await;
                }
                LuaTask::ToggleCaughtFishAll {} => {
                    let max_caught_fish_id = CAUGHT_FISH_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_caught_fish_id {
                        self.toggle_caught_fish(i).await;
                    }
                }
                LuaTask::ToggleCaughtSpearfish { id } => {
                    self.toggle_caught_spearfish(*id).await;
                }
                LuaTask::ToggleCaughtSpearfishAll {} => {
                    let max_caught_spearfish_id = CAUGHT_SPEARFISH_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_caught_spearfish_id {
                        self.toggle_caught_spearfish(i).await;
                    }
                }
                LuaTask::ToggleTripleTriadCard { id } => {
                    self.toggle_triple_triad_card(*id).await;
                }
                LuaTask::ToggleTripleTriadCardAll {} => {
                    let max_triple_triad_card_id = TRIPLE_TRIAD_CARDS_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_triple_triad_card_id {
                        self.toggle_triple_triad_card(i).await;
                    }
                }
                LuaTask::ToggleAdventure { id } => {
                    self.toggle_adventure(*id, false).await;
                }
                LuaTask::ToggleAdventureAll {} => {
                    let max_adventure_id = ADVENTURE_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_adventure_id {
                        if i == 0 {
                            self.toggle_adventure(i, true).await;
                        } else {
                            self.toggle_adventure(i, false).await;
                        }
                    }
                }
                LuaTask::ToggleCutsceneSeen { id, value } => {
                    self.toggle_cutscene_seen(*id, *value).await;
                }
                LuaTask::ToggleCutsceneSeenAll {} => {
                    let max_cutscene_seen_id = CUTSCENE_SEEN_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_cutscene_seen_id {
                        self.toggle_cutscene_seen(i, true).await;
                    }
                }
                LuaTask::ToggleMinion { id } => {
                    self.toggle_minion(*id).await;
                }
                LuaTask::ToggleMinionAll {} => {
                    let max_minion_id = MINION_BITMASK_SIZE as u32 * 8;

                    for i in 0..max_minion_id {
                        self.toggle_minion(i).await;
                    }
                }
                LuaTask::ToggleAetherCurrent { id } => {
                    self.toggle_aether_current(*id).await;
                }
                LuaTask::ToggleAetherCurrentAll {} => {
                    let max_aether_current_id = AETHER_CURRENT_BITMASK_SIZE as u32 * 8;

                    for i in 2818048..(2818048 + max_aether_current_id) {
                        self.toggle_aether_current(i).await;
                    }
                }
                LuaTask::ToggleAetherCurrentCompFlgSet { id } => {
                    self.toggle_aether_current_comp_flg_set(*id).await;
                }
                LuaTask::ToggleAetherCurrentCompFlgSetAll {} => {
                    let max_aether_current_comp_flg_set_id =
                        AETHER_CURRENT_COMP_FLG_SET_BITMASK_SIZE as u32 * 8;

                    // AetherCurrentCompFlgSet starts at Index 1
                    for i in 1..max_aether_current_comp_flg_set_id {
                        self.toggle_aether_current_comp_flg_set(i).await;
                    }
                }
                LuaTask::SetRace { race } => {
                    {
                        let mut database = self.database.lock();
                        let mut chara_make =
                            database.get_chara_make(self.player_data.character.content_id as u64);
                        chara_make.customize.race = *race;

                        database.set_chara_make(
                            self.player_data.character.content_id as u64,
                            &chara_make.to_json(),
                        );
                    }
                    self.respawn_player(false).await;
                }
                LuaTask::SetTribe { tribe } => {
                    {
                        let mut database = self.database.lock();
                        let mut chara_make =
                            database.get_chara_make(self.player_data.character.content_id as u64);
                        chara_make.customize.subrace = *tribe;

                        database.set_chara_make(
                            self.player_data.character.content_id as u64,
                            &chara_make.to_json(),
                        );
                    }
                    self.respawn_player(false).await;
                }
                LuaTask::SetSex { sex } => {
                    {
                        let mut database = self.database.lock();
                        let mut chara_make =
                            database.get_chara_make(self.player_data.character.content_id as u64);
                        chara_make.customize.gender = *sex;

                        database.set_chara_make(
                            self.player_data.character.content_id as u64,
                            &chara_make.to_json(),
                        );
                    }
                    self.respawn_player(false).await;
                }
                LuaTask::SendSegment { segment } => {
                    self.send_segment(segment.clone()).await;
                }
                LuaTask::StartTalkEvent {} => {
                    if let Some(event) = events.last_mut() {
                        event
                            .0
                            .on_talk(
                                &event.1,
                                ObjectTypeId {
                                    object_id: self.player_data.character.actor_id,
                                    object_type: ObjectTypeKind::None,
                                },
                                player,
                            )
                            .await;

                        continue_nesting = true;
                    }
                }
                LuaTask::AcceptQuest { id } => {
                    self.accept_quest(*id).await;
                }
                LuaTask::FinishQuest { id } => {
                    // this means "all"
                    if *id == 65535 {
                        self.finish_all_quests().await;
                    } else {
                        self.finish_quest(*id).await;
                    }
                }
                LuaTask::GainStatusEffect {
                    effect_id,
                    effect_param,
                    duration,
                } => {
                    self.gain_effect(*effect_id, *effect_param, *duration).await;
                }
                LuaTask::RegisterForContent { content_id } => {
                    self.register_for_content([*content_id, 0, 0, 0, 0]).await;
                }
                LuaTask::CommenceDuty { director_id } => {
                    // Have the director commence the duty
                    let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(
                        ActorControlSelf {
                            category: ActorControlCategory::DirectorEvent {
                                handler_id: HandlerId(*director_id),
                                event: DirectorEvent::DutyCommence,
                                arg1: player.content_data.duration as u32,
                                arg2: 0,
                                arg3: 0,
                                arg4: 0,
                            },
                        },
                    ));
                    self.send_ipc_self(ipc).await;

                    // Signal to the global server to commence the duty as well, since they need to update the entrance circle.
                    self.handle
                        .send(ToServer::CommenceDuty(self.player_data.character.actor_id))
                        .await;

                    // shit
                    let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(
                        ActorControlSelf {
                            category: ActorControlCategory::DirectorEvent {
                                handler_id: HandlerId(*director_id),
                                event: DirectorEvent::SetDutyTimeRemaining,
                                arg1: (player.content_data.duration - 1) as u32, // TODO: lol
                                arg2: 0,
                                arg3: 0,
                                arg4: 0,
                            },
                        },
                    ));
                    self.send_ipc_self(ipc).await;
                }
                LuaTask::QuestSequence { id, sequence } => {
                    self.set_quest_sequence(*id, *sequence).await;
                }
                LuaTask::CancelQuest { id } => {
                    self.cancel_quest(*id).await;
                }
                LuaTask::IncompleteQuest { id } => {
                    // this means "all"
                    if *id == 65535 {
                        self.incomplete_all_quests().await;
                    } else {
                        self.incomplete_quest(*id).await;
                    }
                }
                LuaTask::Kill {} => {
                    // Signal to the global server to kill us.
                    self.handle
                        .send(ToServer::Kill(self.id, self.player_data.character.actor_id))
                        .await;
                }
                LuaTask::AbandonContent {} => {
                    // Signal to the global server to leave this content.
                    self.handle
                        .send(ToServer::LeaveContent(
                            self.id,
                            self.player_data.character.actor_id,
                            self.old_zone_id,
                            self.old_position,
                            self.old_rotation,
                        ))
                        .await;
                }
                LuaTask::SetHomepoint { homepoint } => {
                    self.player_data.aetheryte.homepoint = *homepoint as i32;

                    // Also update the client live
                    self.actor_control_self(ActorControlCategory::SetHomepoint {
                        id: *homepoint as u32,
                    })
                    .await;
                }
                LuaTask::ReturnToHomepoint {} => {
                    self.warp_aetheryte(self.player_data.aetheryte.homepoint as u32, false, false)
                        .await;
                }
                LuaTask::JoinContent { id } => {
                    self.join_content(*id as u16).await;
                }
                LuaTask::FinishCastingGlamour {} => {
                    // NOTE: Needs a replay from retail, I guessed here because TBH who manually casts glamours anymore

                    if let Some(ClientTriggerCommand::PrepareCastGlamour {
                        dst_container_type,
                        dst_container_index,
                        src_container_type,
                        src_container_index,
                    }) = self.glamour_information
                    {
                        let Some(src_slot) = self
                            .player_data
                            .inventory
                            .get_item(src_container_type, src_container_index as u16)
                        else {
                            return true;
                        };
                        let Some(dst_slot) = self
                            .player_data
                            .inventory
                            .get_item_mut(dst_container_type, dst_container_index as u16)
                        else {
                            return true;
                        };

                        dst_slot.glamour_id = src_slot.item_id;

                        // The client needs to be informed about the new glamoured item, but this is extreme...
                        self.send_inventory().await;
                        self.inform_equip().await;

                        self.send_conditions().await; // So the client gets unstuck.
                    }
                    if let Some(ClientTriggerCommand::PrepareRemoveGlamour {
                        dst_container_type,
                        dst_container_index,
                    }) = self.glamour_information
                    {
                        let Some(dst_slot) = self
                            .player_data
                            .inventory
                            .get_item_mut(dst_container_type, dst_container_index as u16)
                        else {
                            return true;
                        };

                        dst_slot.glamour_id = 0;

                        // The client needs to be informed about the new glamoured item, but this is extreme...
                        self.send_inventory().await;
                        self.inform_equip().await;

                        self.send_conditions().await; // So the client gets unstuck.
                    }

                    // Reset information so it's not accidentally reused.
                    self.glamour_information = None;
                }
                LuaTask::PlayScene {
                    scene,
                    scene_flags,
                    params,
                } => {
                    self.event_scene(
                        &events.last().unwrap().1,
                        *scene,
                        *scene_flags,
                        params.clone(),
                    )
                    .await;
                }
                LuaTask::ResumeEvent {
                    scene,
                    resume_id,
                    params,
                } => {
                    self.resume_event(
                        self.event_handler_id.unwrap().0,
                        *scene,
                        *resume_id,
                        params.clone(),
                    )
                    .await;
                }
                LuaTask::WarpPopRange {
                    territory_id,
                    pop_range_id,
                } => {
                    self.handle
                        .send(ToServer::WarpPopRange(
                            self.id,
                            self.player_data.character.actor_id,
                            *territory_id,
                            *pop_range_id,
                        ))
                        .await;
                }
                LuaTask::RemoveCooldowns {} => {
                    self.handle
                        .send(ToServer::RemoveCooldowns(
                            self.player_data.character.actor_id,
                        ))
                        .await;
                }
                LuaTask::ToggleHowTo { value, id } => {
                    if *value {
                        self.player_data.unlock.seen_active_help.clear(*id);
                    } else {
                        self.player_data.unlock.seen_active_help.set(*id);
                    }
                }
                LuaTask::SendMailboxStatus {} => {
                    self.send_mailbox_status().await;
                }
                LuaTask::SetGrandCompany { company } => {
                    self.set_grand_company(*company);
                    self.send_grand_company_info().await;
                }
                LuaTask::SetGrandCompanyRank { rank } => {
                    self.set_grand_company_rank(*rank);
                    self.send_grand_company_info().await;
                }
                LuaTask::Jump { name } => {
                    self.handle
                        .send(ToServer::Jump(self.id, name.clone()))
                        .await;
                }
                LuaTask::Call { name } => {
                    self.handle
                        .send(ToServer::Call(
                            self.player_data.character.actor_id,
                            name.clone(),
                        ))
                        .await;
                }
            }
        }

        // We want to process again, since we probably added more tasks.
        // If we *don't* do this there is a pretty big delay before this can happen again.
        if run_finish_event {
            // To handle PreHandler's & others nesting, which is probably not going to scale.
            self.event_finish(events).await;

            return true;
        }

        continue_nesting
    }

    /// Reloads Global.lua
    pub async fn reload_scripts(&mut self) {
        {
            let mut lua = self.lua.lock();
            if let Err(err) = lua.init(self.gamedata.clone()) {
                tracing::warn!("Failed to load Init.lua: {:?}", err);
            }
        }

        // Then inform the server state to reload its own state as well
        self.handle.send(ToServer::ReloadScripts).await;
    }

    fn current_or_owned_housing_estate(&mut self) -> Option<HousingEstate> {
        self.selected_or_owned_housing_estate()
    }
}

fn housing_kit_items(kit: HousingKit) -> &'static [u32] {
    match kit {
        HousingKit::Indoor => &[6514, 6607, 6635, 6657, 6674, 6578, 7064],
        HousingKit::Outdoor => &[6475, 6484, 6486, 6490, 7128, 7118, 12113],
        HousingKit::Npc => &[7064, 23846, 9748, 9749],
    }
}

#[derive(Debug)]
struct RemakePlacePresetAnalysis {
    preset_path: RemakePlacePresetPath,
    import_rows: RemakePlaceImportRows,
    fixture_updates: RemakePlaceInteriorFixtureUpdates,
}

#[derive(Debug)]
struct RemakePlacePresetApplyOutcome {
    path: PathBuf,
    summary: String,
    notice: String,
}

fn apply_remake_place_preset_to_estate(
    connection: &mut ZoneConnection,
    estate: &HousingEstate,
    preset_path: RemakePlacePresetPath,
    scope: HousingPresetScope,
    reload: bool,
) -> Result<RemakePlacePresetApplyOutcome, String> {
    tracing::debug!(
        path = %display_remake_place_preset_path(&preset_path),
        land_ident = estate.land_ident,
        scope = housing_preset_scope_label(scope),
        "Analyzing ReMakePlace housing preset"
    );
    let analysis = analyze_remake_place_preset_for_estate(connection, estate, preset_path, scope)?;
    let preset_path = analysis.preset_path;
    let import_rows = analysis.import_rows;
    let fixture_updates = analysis.fixture_updates;

    tracing::debug!(
        path = %display_remake_place_preset_path(&preset_path),
        land_ident = estate.land_ident,
        rows = import_rows.rows.len(),
        indoor = import_rows.indoor_imported,
        outdoor = import_rows.outdoor_imported,
        fixtures = fixture_updates.fixture_updates.len(),
        style = fixture_updates.renovation_row_id.unwrap_or_default(),
        "Persisting ReMakePlace housing preset"
    );
    let deleted = {
        let mut database = connection.database.lock();
        let deleted = database
            .replace_housing_placed_furniture_for_estate(
                estate.land_ident,
                scope.includes_interior(),
                scope.includes_exterior(),
                &import_rows.rows,
            )
            .map_err(|error| format!("Unable to apply ReMakePlace preset to database: {error}"))?;

        if scope.includes_interior()
            && (fixture_updates.renovation_row_id.is_some()
                || !fixture_updates.fixture_updates.is_empty())
        {
            let mut interior_json = estate.interior_json.clone();
            if let Some(renovation_row_id) = fixture_updates.renovation_row_id {
                interior_json =
                    update_interior_json_renovation_row_id(&interior_json, renovation_row_id)
                        .map_err(|error| {
                            format!("Unable to update ReMakePlace interior style: {error}")
                        })?;
            }

            for (field, value) in &fixture_updates.fixture_updates {
                interior_json = update_interior_json_field(&interior_json, *field, *value)
                    .map_err(|error| {
                        format!("Unable to update ReMakePlace interior fixture {field:?}: {error}")
                    })?;
            }

            if !database.update_housing_interior_json(estate.land_ident, &interior_json) {
                return Err("Unable to persist ReMakePlace interior fixtures.".to_string());
            }
        }

        deleted
    };
    connection.clear_housing_furniture_reset_cache();
    tracing::debug!(
        path = %display_remake_place_preset_path(&preset_path),
        land_ident = estate.land_ident,
        deleted,
        "Persisted ReMakePlace housing preset"
    );

    let path = preset_path.path.clone();
    let summary = format_remake_place_preset_summary(
        "Applied",
        &preset_path,
        estate,
        scope,
        &import_rows,
        &fixture_updates,
        Some(deleted),
        "Use !housing reload or re-enter the estate/ward to refresh visuals.",
    );
    let notice = format_remake_place_preset_notice(
        "Applied",
        &preset_path,
        scope,
        &import_rows,
        &fixture_updates,
        Some(deleted),
        reload,
    );
    tracing::debug!(
        path = %display_remake_place_preset_path(&preset_path),
        summary_len = summary.len(),
        "Built ReMakePlace housing preset summary"
    );

    Ok(RemakePlacePresetApplyOutcome {
        path,
        summary,
        notice,
    })
}

fn check_remake_place_preset_for_estate(
    connection: &mut ZoneConnection,
    estate: &HousingEstate,
    preset_path: RemakePlacePresetPath,
    scope: HousingPresetScope,
) -> Result<String, String> {
    tracing::debug!(
        path = %display_remake_place_preset_path(&preset_path),
        land_ident = estate.land_ident,
        scope = housing_preset_scope_label(scope),
        "Analyzing ReMakePlace housing preset check"
    );
    let analysis = analyze_remake_place_preset_for_estate(connection, estate, preset_path, scope)?;
    tracing::debug!(
        path = %display_remake_place_preset_path(&analysis.preset_path),
        land_ident = estate.land_ident,
        rows = analysis.import_rows.rows.len(),
        indoor = analysis.import_rows.indoor_imported,
        outdoor = analysis.import_rows.outdoor_imported,
        fixtures = analysis.fixture_updates.fixture_updates.len(),
        style = analysis.fixture_updates.renovation_row_id.unwrap_or_default(),
        "Checked ReMakePlace housing preset"
    );

    Ok(format_remake_place_preset_summary(
        "Checked",
        &analysis.preset_path,
        estate,
        scope,
        &analysis.import_rows,
        &analysis.fixture_updates,
        None,
        "No database changes were made.",
    ))
}

fn analyze_remake_place_preset_for_estate(
    connection: &mut ZoneConnection,
    estate: &HousingEstate,
    preset_path: RemakePlacePresetPath,
    scope: HousingPresetScope,
) -> Result<RemakePlacePresetAnalysis, String> {
    let layout = parse_remake_place_layout_file(&preset_path.path)?;
    let created_by_content_id = Some(connection.player_data.character.content_id as i64);
    let plot_size = PlotSize::from_repr(estate.plot_size as u8).unwrap_or(PlotSize::Large);
    let (import_rows, fixture_updates) = {
        let mut game_data = connection.gamedata.lock();
        let import_rows = build_remake_place_furniture_rows(
            &layout,
            estate.land_ident,
            created_by_content_id,
            |item_id| game_data.get_furniture_catalog_id(item_id),
            |rgb| game_data.get_closest_housing_stain(rgb),
            scope,
        );
        let fixture_updates = if scope.includes_interior() {
            build_remake_place_interior_fixture_updates(&layout, plot_size, |item_id| {
                game_data
                    .get_item_info(ItemInfoQuery::ById(item_id))
                    .map(|item| (item.additional_data, item.item_ui_category))
            })
        } else {
            Default::default()
        };
        (import_rows, fixture_updates)
    };

    Ok(RemakePlacePresetAnalysis {
        preset_path,
        import_rows,
        fixture_updates,
    })
}

fn format_remake_place_preset_summary(
    action: &str,
    preset_path: &RemakePlacePresetPath,
    estate: &HousingEstate,
    scope: HousingPresetScope,
    import_rows: &RemakePlaceImportRows,
    fixture_updates: &RemakePlaceInteriorFixtureUpdates,
    replaced: Option<usize>,
    suffix: &str,
) -> String {
    format!(
        "{action} ReMakePlace preset {} to {} ({}): indoor={} outdoor={} fixtures={} style={} replaced={} skipped missing_item={} missing_catalog={} capacity={} fixture_missing_item={} fixture_missing_data={} fixture_wrong_category={}. {suffix}",
        display_remake_place_preset_path(preset_path),
        estate.estate_name,
        housing_preset_scope_label(scope),
        import_rows.indoor_imported,
        import_rows.outdoor_imported,
        fixture_updates.fixture_updates.len(),
        fixture_updates
            .renovation_row_id
            .map(|row_id| row_id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        replaced
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
        import_rows.skipped_missing_item_id,
        import_rows.skipped_missing_catalog,
        import_rows.skipped_capacity,
        fixture_updates.skipped_missing_item_id,
        fixture_updates.skipped_missing_item_data,
        fixture_updates.skipped_wrong_category,
    )
}

fn format_remake_place_preset_notice(
    action: &str,
    preset_path: &RemakePlacePresetPath,
    scope: HousingPresetScope,
    import_rows: &RemakePlaceImportRows,
    fixture_updates: &RemakePlaceInteriorFixtureUpdates,
    replaced: Option<usize>,
    reload: bool,
) -> String {
    let refresh = if reload {
        "Reloading housing."
    } else {
        "Use !housing reload to refresh."
    };
    format!(
        "{action} ReMakePlace preset {} ({}): indoor={} outdoor={} fixtures={} style={} replaced={}. {refresh}",
        display_remake_place_preset_path(preset_path),
        housing_preset_scope_label(scope),
        import_rows.indoor_imported,
        import_rows.outdoor_imported,
        fixture_updates.fixture_updates.len(),
        fixture_updates
            .renovation_row_id
            .map(|row_id| row_id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        replaced
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
    )
}

fn display_remake_place_preset_path(preset_path: &RemakePlacePresetPath) -> String {
    preset_path
        .path
        .strip_prefix(&preset_path.root)
        .unwrap_or(&preset_path.path)
        .display()
        .to_string()
}

async fn reload_housing_after_preset(connection: &mut ZoneConnection, scope: HousingPresetScope) {
    tracing::debug!(
        scope = housing_preset_scope_label(scope),
        "Reloading housing after ReMakePlace preset"
    );
    if matches!(scope, HousingPresetScope::Exterior) {
        connection.exit_local_house().await;
    } else {
        connection.reload_current_housing_interior().await;
    }
}

fn housing_preset_scope_label(scope: HousingPresetScope) -> &'static str {
    match scope {
        HousingPresetScope::All => "all",
        HousingPresetScope::Interior => "interior",
        HousingPresetScope::Exterior => "exterior",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HousingFixtureUpdateResult {
    Updated,
    MissingEstate,
    InvalidStoredJson,
}

fn apply_housing_exterior_fixture_update(
    database: &mut WorldDatabase,
    estate: &HousingEstate,
    field: HousingExteriorField,
    value: u16,
) -> HousingFixtureUpdateResult {
    match update_exterior_json_field(&estate.exterior_json, field, value) {
        Ok(exterior_json) => {
            if database.update_housing_exterior_json(estate.land_ident, &exterior_json) {
                HousingFixtureUpdateResult::Updated
            } else {
                HousingFixtureUpdateResult::MissingEstate
            }
        }
        Err(_) => HousingFixtureUpdateResult::InvalidStoredJson,
    }
}

fn apply_housing_exterior_color_update(
    database: &mut WorldDatabase,
    estate: &HousingEstate,
    field: HousingExteriorColorField,
    value: u8,
) -> HousingFixtureUpdateResult {
    match update_exterior_json_color(&estate.exterior_json, field, value) {
        Ok(exterior_json) => {
            if database.update_housing_exterior_json(estate.land_ident, &exterior_json) {
                HousingFixtureUpdateResult::Updated
            } else {
                HousingFixtureUpdateResult::MissingEstate
            }
        }
        Err(_) => HousingFixtureUpdateResult::InvalidStoredJson,
    }
}

fn apply_housing_interior_fixture_update(
    database: &mut WorldDatabase,
    estate: &HousingEstate,
    field: HousingInteriorField,
    value: u32,
) -> HousingFixtureUpdateResult {
    match update_interior_json_field(&estate.interior_json, field, value) {
        Ok(interior_json) => {
            if database.update_housing_interior_json(estate.land_ident, &interior_json) {
                HousingFixtureUpdateResult::Updated
            } else {
                HousingFixtureUpdateResult::MissingEstate
            }
        }
        Err(_) => HousingFixtureUpdateResult::InvalidStoredJson,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        HousingFixtureUpdateResult, apply_housing_exterior_color_update,
        apply_housing_exterior_fixture_update, apply_housing_interior_fixture_update,
        format_remake_place_preset_notice,
    };
    use crate::{
        database::WorldDatabase,
        lua::{HousingExteriorColorField, HousingExteriorField, HousingInteriorField, LuaTask},
        zone_connection::remake_place::{
            RemakePlaceImportRows, RemakePlaceInteriorFixtureUpdates, RemakePlacePresetPath,
        },
    };
    use kawari::common::HouseId;

    fn run_malformed_update_task(task: LuaTask, target_field_json: &str) {
        let mut database = WorldDatabase::new_at(":memory:");
        let estate = database.ensure_local_estate(100, "Tester", 67);
        let before = database
            .housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
            .expect("ensured house should exist");
        let expected_json = match &task {
            LuaTask::UpdateHousingExterior { .. } | LuaTask::UpdateHousingExteriorColor { .. } => {
                before.exterior_json.clone()
            }
            LuaTask::UpdateHousingInterior { .. } => before.interior_json.clone(),
            _ => unreachable!(),
        };
        assert_ne!(
            expected_json, target_field_json,
            "seeded fixture should be malformed JSON"
        );
        let expected_before = target_field_json.to_string();

        let malformed_seeded = match &task {
            LuaTask::UpdateHousingExterior { .. } | LuaTask::UpdateHousingExteriorColor { .. } => {
                database.update_housing_exterior_json(estate.land_ident, target_field_json)
            }
            LuaTask::UpdateHousingInterior { .. } => {
                database.update_housing_interior_json(estate.land_ident, target_field_json)
            }
            _ => unreachable!(),
        };
        assert!(malformed_seeded);
        let selected = database
            .owned_housing_estates(100)
            .into_iter()
            .find(|estate| estate.owner_content_id == Some(100))
            .expect("owned estate should still be available");
        let outcome = match task {
            LuaTask::UpdateHousingExterior { field, value } => {
                apply_housing_exterior_fixture_update(&mut database, &selected, field, value)
            }
            LuaTask::UpdateHousingExteriorColor { field, value } => {
                apply_housing_exterior_color_update(&mut database, &selected, field, value)
            }
            LuaTask::UpdateHousingInterior { field, value } => {
                apply_housing_interior_fixture_update(&mut database, &selected, field, value)
            }
            _ => unreachable!(),
        };
        assert_eq!(outcome, HousingFixtureUpdateResult::InvalidStoredJson);

        let updated_estate = database
            .housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
            .unwrap();
        match task {
            LuaTask::UpdateHousingExterior { .. } | LuaTask::UpdateHousingExteriorColor { .. } => {
                assert_eq!(updated_estate.exterior_json, expected_before);
            }
            LuaTask::UpdateHousingInterior { .. } => {
                assert_eq!(updated_estate.interior_json, expected_before);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn update_housing_exterior_no_overwrite_when_json_is_malformed() {
        let task = LuaTask::UpdateHousingExterior {
            field: HousingExteriorField::Roof,
            value: 9,
        };
        run_malformed_update_task(task, "{");
    }

    #[test]
    fn update_housing_exterior_color_no_overwrite_when_json_is_malformed() {
        let task = LuaTask::UpdateHousingExteriorColor {
            field: HousingExteriorColorField::Roof,
            value: 3,
        };
        run_malformed_update_task(task, "{");
    }

    #[test]
    fn update_housing_interior_no_overwrite_when_json_is_malformed() {
        let task = LuaTask::UpdateHousingInterior {
            field: HousingInteriorField::WindowStyle,
            value: 123,
        };
        run_malformed_update_task(task, "{");
    }

    #[test]
    fn remake_place_preset_notice_stays_short_for_large_imports() {
        let preset_path = RemakePlacePresetPath {
            root: PathBuf::from(r"D:\ReMakePlace_Latest\MakePlace\Save"),
            path: PathBuf::from(r"D:\ReMakePlace_Latest\MakePlace\Save\CL03 Meridian Neue L.json"),
        };
        let import_rows = RemakePlaceImportRows {
            rows: Vec::new(),
            indoor_imported: 596,
            outdoor_imported: 0,
            skipped_missing_item_id: 0,
            skipped_missing_catalog: 0,
            skipped_capacity: 0,
        };
        let fixture_updates = RemakePlaceInteriorFixtureUpdates {
            renovation_row_id: Some(18),
            fixture_updates: vec![
                (HousingInteriorField::GroundChandelier, 1),
                (HousingInteriorField::TopChandelier, 2),
            ],
            ..Default::default()
        };

        let notice = format_remake_place_preset_notice(
            "Applied",
            &preset_path,
            crate::lua::HousingPresetScope::Interior,
            &import_rows,
            &fixture_updates,
            Some(596),
            true,
        );

        assert!(notice.len() < 180, "{notice}");
        assert!(notice.contains("Applied ReMakePlace preset CL03 Meridian Neue L.json"));
        assert!(notice.contains("indoor=596"));
        assert!(notice.contains("fixtures=2"));
        assert!(notice.contains("style=18"));
        assert!(notice.contains("replaced=596"));
        assert!(notice.contains("Reloading housing"));
    }
}
