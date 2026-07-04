use std::sync::Arc;

use mlua::{LuaSerdeExt, UserData, UserDataFields, UserDataMethods, Value};
use parking_lot::Mutex;

use crate::{
    GameData, PlayerData, RemakeMode, StatusEffects,
    housing::apartment::valid_apartment_room_number,
    inventory::{CrystalKind, CurrencyKind},
    zone_connection::{ActiveHousingWardContext, BaseParameters},
};
use kawari::{
    common::{HandlerId, ObjectTypeId, ObjectTypeKind, Position, adjust_quest_id},
    ipc::zone::{
        ActorControlCategory, ActorControlSelf, ActorSetPos, EventType, GrandCompany, OnlineStatus,
        PlotSize, SceneFlags, ServerNoticeFlags, ServerNoticeMessage, ServerZoneIpcData,
        ServerZoneIpcSegment,
    },
    packet::PacketSegment,
};

use super::housing_placard_location_from_event_arg;
use super::{
    HousingEstateKind, HousingExteriorColorField, HousingExteriorField, HousingInteriorField,
    HousingKit, HousingPresetScope, HousingResetMode, LuaTask, LuaZone, QueueSegments,
    create_ipc_self,
};

#[derive(Default, Clone, Copy)]
pub struct LuaContent {
    /// Duration in seconds.
    pub duration: u16,
    /// Duty finder settings. See ContentRegistrationFlags.
    pub settings: u32,
}

impl UserData for LuaContent {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("duration", |_, this| Ok(this.duration));

        fields.add_field_method_get("settings", |_, this| Ok(this.settings));
    }
}

#[derive(Default)]
pub struct LuaPlayer {
    pub player_data: PlayerData,
    pub queued_tasks: Vec<LuaTask>,
    pub zone_data: LuaZone,
    pub status_effects: StatusEffects,
    pub content_data: LuaContent,
    // TODO: move this into PlayerData
    pub base_parameters: BaseParameters,
    pub housing_ward_context: ActiveHousingWardContext,
}

impl QueueSegments for LuaPlayer {
    fn queue_segment(&mut self, segment: PacketSegment<ServerZoneIpcSegment>) {
        self.queued_tasks.push(LuaTask::SendSegment { segment });
    }
}

impl LuaPlayer {
    fn send_message(&mut self, message: &str, param: u8) {
        // This is a completely arbitrary string, so we have to make sure it's the proper size.
        let mut message = message.to_string();
        message.truncate(775);

        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::ServerNoticeMessage(
            ServerNoticeMessage {
                message,
                flags: ServerNoticeFlags::from_bits(param).unwrap_or_default(),
            },
        ));

        create_ipc_self(self, ipc, self.player_data.character.actor_id);
    }

    fn give_status_effect(&mut self, effect_id: u16, effect_param: u16, duration: f32) {
        self.queued_tasks.push(LuaTask::GainStatusEffect {
            effect_id,
            effect_param,
            duration,
        });
    }

    pub fn play_scene(&mut self, scene: u16, scene_flags: SceneFlags, params: Vec<u32>) {
        self.queued_tasks.push(LuaTask::PlayScene {
            scene,
            scene_flags,
            params,
        });
    }

    fn set_position(&mut self, position: Position, rotation: f32) {
        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::ActorSetPos(ActorSetPos {
            rotation,
            position,
            ..Default::default()
        }));

        create_ipc_self(self, ipc, self.player_data.character.actor_id);
    }

    fn set_festival(&mut self, festival1: u32, festival2: u32, festival3: u32, festival4: u32) {
        let ipc =
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::SetFestival {
                    festival1,
                    festival2,
                    festival3,
                    festival4,
                },
            }));

        create_ipc_self(self, ipc, self.player_data.character.actor_id);
    }

    fn unlock(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::Unlock { id });
    }

    fn unlock_all(&mut self) {
        self.queued_tasks.push(LuaTask::UnlockAll {});
    }

    fn set_speed(&mut self, speed: u16) {
        let ipc =
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::Flee { speed },
            }));

        create_ipc_self(self, ipc, self.player_data.character.actor_id);
    }

    fn toggle_wireframe(&mut self) {
        let ipc =
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::ToggleWireframeRendering(),
            }));

        create_ipc_self(self, ipc, self.player_data.character.actor_id);
    }

    fn unlock_aetheryte(&mut self, unlocked: u32, id: u32) {
        self.queued_tasks.push(LuaTask::UnlockAetheryte {
            id,
            on: unlocked == 1,
        });
    }

    fn change_territory(
        &mut self,
        zone_id: u16,
        exit_position: Option<Position>,
        exit_rotation: Option<f32>,
    ) {
        self.queued_tasks.push(LuaTask::ChangeTerritory {
            zone_id,
            exit_position,
            exit_rotation,
        });
    }

    fn set_remake_mode(&mut self, mode: RemakeMode) {
        self.queued_tasks.push(LuaTask::SetRemakeMode(mode));
    }

    fn warp(&mut self, warp_id: u32) {
        self.queued_tasks.push(LuaTask::Warp { warp_id });
    }

    fn begin_log_out(&mut self) {
        self.queued_tasks.push(LuaTask::BeginLogOut);
    }

    pub fn finish_event(&mut self) {
        self.queued_tasks.push(LuaTask::FinishEvent {});
    }

    fn unlock_classjob(&mut self, classjob_id: u8) {
        self.queued_tasks
            .push(LuaTask::UnlockClassJob { classjob_id });
    }

    fn warp_aetheryte(&mut self, aetheryte_id: u32, housing_aethernet: bool) {
        self.queued_tasks.push(LuaTask::WarpAetheryte {
            aetheryte_id,
            housing_aethernet,
        });
    }

    fn toggle_invisiblity(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleInvisibility {
            invisible: !self.player_data.gm_invisible,
        });
    }

    fn set_level(&mut self, level: u16) {
        self.queued_tasks.push(LuaTask::SetLevel { level });
    }

    fn change_weather(&mut self, id: u8) {
        self.queued_tasks.push(LuaTask::ChangeWeather { id });
    }

    pub fn modify_currency(&mut self, id: CurrencyKind, amount: i32, send_client_update: bool) {
        self.queued_tasks.push(LuaTask::ModifyCurrency {
            id,
            amount,
            send_client_update,
        });
    }

    pub fn modify_crystal(&mut self, id: CrystalKind, amount: i32, send_client_update: bool) {
        self.queued_tasks.push(LuaTask::ModifyCrystal {
            id,
            amount,
            send_client_update,
        });
    }

    fn gm_set_orchestrion(&mut self, value: bool, id: u32) {
        self.queued_tasks
            .push(LuaTask::GmSetOrchestrion { value, id });
    }

    fn toggle_orchestrion(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleOrchestrion { id });
    }

    pub fn add_item(&mut self, id: u32, quantity: u32, send_client_update: bool) {
        self.queued_tasks.push(LuaTask::AddItem {
            id,
            quantity,
            send_client_update,
        });
    }

    fn show_housing_placard(&mut self, ward_index: u8, division: u8, plot_index: u8) {
        self.queued_tasks.push(LuaTask::ShowHousingPlacard {
            ward_index,
            division,
            plot_index,
        });
    }

    fn ensure_local_apartment(&mut self, room_number: u16) {
        if !valid_apartment_room_number(room_number) {
            return;
        }

        self.queued_tasks
            .push(LuaTask::EnsureLocalApartment { room_number });
    }

    fn ensure_local_house(&mut self) {
        self.queued_tasks.push(LuaTask::EnsureLocalHouse {});
    }

    fn ensure_local_house_with_options(
        &mut self,
        kind: HousingEstateKind,
        size: PlotSize,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: u8,
    ) {
        self.queued_tasks
            .push(LuaTask::EnsureLocalHouseWithOptions {
                kind,
                size,
                territory_type_id,
                ward_index,
                division,
                plot_index,
            });
    }

    fn reset_housing(&mut self, mode: HousingResetMode) {
        self.queued_tasks.push(LuaTask::ResetHousing { mode });
    }

    fn update_housing_name(&mut self, name: String) {
        self.queued_tasks.push(LuaTask::UpdateHousingName { name });
    }

    fn update_housing_greeting(&mut self, greeting: String) {
        self.queued_tasks
            .push(LuaTask::UpdateHousingGreeting { greeting });
    }

    fn update_housing_light(&mut self, level: u8) {
        self.queued_tasks
            .push(LuaTask::UpdateHousingLight { level });
    }

    fn update_housing_exterior(&mut self, field: HousingExteriorField, value: u16) {
        self.queued_tasks
            .push(LuaTask::UpdateHousingExterior { field, value });
    }

    fn update_housing_exterior_color(&mut self, field: HousingExteriorColorField, value: u8) {
        self.queued_tasks
            .push(LuaTask::UpdateHousingExteriorColor { field, value });
    }

    fn update_housing_interior(&mut self, field: HousingInteriorField, value: u32) {
        self.queued_tasks
            .push(LuaTask::UpdateHousingInterior { field, value });
    }

    fn apply_housing_preset(&mut self, path: String, scope: HousingPresetScope, reload: bool) {
        self.queued_tasks.push(LuaTask::ApplyHousingPreset {
            path,
            scope,
            reload,
        });
    }

    fn apply_latest_housing_preset(&mut self, scope: HousingPresetScope, reload: bool) {
        self.queued_tasks
            .push(LuaTask::ApplyLatestHousingPreset { scope, reload });
    }

    fn repeat_housing_preset(&mut self, reload: bool) {
        self.queued_tasks
            .push(LuaTask::RepeatHousingPreset { reload });
    }

    fn check_housing_preset(&mut self, path: String, scope: HousingPresetScope) {
        self.queued_tasks
            .push(LuaTask::CheckHousingPreset { path, scope });
    }

    fn check_latest_housing_preset(&mut self, scope: HousingPresetScope) {
        self.queued_tasks
            .push(LuaTask::CheckLatestHousingPreset { scope });
    }

    fn check_repeated_housing_preset(&mut self) {
        self.queued_tasks
            .push(LuaTask::CheckRepeatedHousingPreset {});
    }

    fn give_housing_kit(&mut self, kit: HousingKit) {
        self.queued_tasks.push(LuaTask::GiveHousingKit { kit });
    }

    fn enter_local_apartment(&mut self, room_number: u16) {
        if !valid_apartment_room_number(room_number) {
            return;
        }

        self.queued_tasks
            .push(LuaTask::EnterLocalApartment { room_number });
    }

    fn enter_local_house(&mut self) {
        self.queued_tasks.push(LuaTask::EnterLocalHouse {});
    }

    fn exit_local_house(&mut self) {
        self.queued_tasks.push(LuaTask::ExitLocalHouse {});
    }

    fn reload_housing(&mut self) {
        self.queued_tasks.push(LuaTask::ReloadHousing {});
    }

    fn unlock_content(&mut self, id: u16) {
        self.queued_tasks.push(LuaTask::UnlockContent { id });
    }

    fn unlock_all_content(&mut self) {
        self.queued_tasks.push(LuaTask::UnlockAllContent {});
    }

    fn do_solnine_teleporter(
        &mut self,
        event_id: u32,
        path_id: u32,
        unk2: u16,
        unk3: u16,
        speed: u16,
        unk4: u16,
        unk5: u32,
    ) {
        let packets_to_send = [
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::DisableEventPosRollback {
                    handler_id: HandlerId(event_id),
                },
            })),
            ServerZoneIpcSegment::new(ServerZoneIpcData::WalkInEvent {
                path_id,
                unk2,
                unk3,
                speed,
                constant: 1,
                unk4,
                unk5,
            }),
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::MovementRelatedUnk {
                    unk1: 1, // Sometimes the server sends 2 for this, but it's still completely unknown what it means.
                },
            })),
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::SetPetEntityId { unk1: 1 },
            })),
        ];

        for ipc in packets_to_send {
            create_ipc_self(self, ipc, self.player_data.character.actor_id);
        }
    }

    fn add_exp(&mut self, amount: i32) {
        self.queued_tasks.push(LuaTask::AddExp { amount });
    }

    fn start_event(&mut self, event_id: u32, event_type: EventType, event_arg: u32) {
        self.queued_tasks.push(LuaTask::StartEvent {
            event_id,
            event_type,
            event_arg,
        });
    }

    fn set_inn_wakeup(&mut self, watched: bool) {
        self.queued_tasks.push(LuaTask::SetInnWakeup { watched });
    }

    fn toggle_mount(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleMount { id });
    }

    fn toggle_glasses_style(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleGlassesStyle { id });
    }

    fn toggle_glasses_style_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleGlassesStyleAll {});
    }

    fn toggle_ornament(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleOrnament { id });
    }

    fn toggle_ornament_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleOrnamentAll {});
    }

    fn unlock_buddy_equip(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::UnlockBuddyEquip { id });
    }

    fn unlock_buddy_equip_all(&mut self) {
        self.queued_tasks.push(LuaTask::UnlockBuddyEquipAll {});
    }

    fn toggle_chocobo_taxi_stand(&mut self, id: u32) {
        self.queued_tasks
            .push(LuaTask::ToggleChocoboTaxiStand { id });
    }

    fn toggle_chocobo_taxi_stand_all(&mut self) {
        self.queued_tasks
            .push(LuaTask::ToggleChocoboTaxiStandAll {});
    }

    fn toggle_caught_fish(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleCaughtFish { id });
    }

    fn toggle_caught_fish_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleCaughtFishAll {});
    }

    fn toggle_caught_spearfish(&mut self, id: u32) {
        self.queued_tasks
            .push(LuaTask::ToggleCaughtSpearfish { id });
    }

    fn toggle_caught_spearfish_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleCaughtSpearfishAll {});
    }

    fn toggle_triple_triad_card(&mut self, id: u32) {
        self.queued_tasks
            .push(LuaTask::ToggleTripleTriadCard { id });
    }

    fn toggle_triple_triad_card_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleTripleTriadCardAll {});
    }

    fn toggle_adventure(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleAdventure { id });
    }

    fn toggle_adventure_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleAdventureAll {});
    }

    fn toggle_cutscene_seen(&mut self, id: u32, value: bool) {
        self.queued_tasks
            .push(LuaTask::ToggleCutsceneSeen { id, value });
    }

    fn toggle_cutscene_seen_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleCutsceneSeenAll {});
    }

    fn toggle_minion(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleMinion { id });
    }

    fn toggle_minion_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleMinionAll {});
    }

    fn toggle_aether_current(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleAetherCurrent { id });
    }

    fn toggle_aether_current_all(&mut self) {
        self.queued_tasks.push(LuaTask::ToggleAetherCurrentAll {});
    }

    fn toggle_aether_current_comp_flg_set(&mut self, id: u32) {
        self.queued_tasks
            .push(LuaTask::ToggleAetherCurrentCompFlgSet { id });
    }

    fn toggle_aether_current_comp_flg_set_all(&mut self) {
        self.queued_tasks
            .push(LuaTask::ToggleAetherCurrentCompFlgSetAll {});
    }

    fn move_to_pop_range(&mut self, id: u32, fade_out: bool) {
        self.queued_tasks
            .push(LuaTask::MoveToPopRange { id, fade_out });
    }

    fn set_hp(&mut self, hp: u32) {
        self.queued_tasks.push(LuaTask::SetHP { hp });
    }

    fn set_mp(&mut self, mp: u16) {
        self.queued_tasks.push(LuaTask::SetMP { mp });
    }

    fn set_race(&mut self, race: u8) {
        self.queued_tasks.push(LuaTask::SetRace { race });
    }

    fn set_tribe(&mut self, tribe: u8) {
        self.queued_tasks.push(LuaTask::SetTribe { tribe });
    }

    fn set_sex(&mut self, sex: u8) {
        self.queued_tasks.push(LuaTask::SetSex { sex });
    }

    fn start_talk_event(&mut self) {
        self.queued_tasks.push(LuaTask::StartTalkEvent {});
    }

    fn accept_quest(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::AcceptQuest { id });
    }

    fn finish_quest(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::FinishQuest { id });
    }

    pub fn commence_duty(&mut self, director_id: u32) {
        self.queued_tasks
            .push(LuaTask::CommenceDuty { director_id });
    }

    /// Returns the target DefaultTalk event for a given SwitchTalk event.
    /// This takes quest completion into account.
    fn get_switch_talk_target(
        &mut self,
        game_data: mlua::Value,
        switch_talk_id: u32,
    ) -> Option<u32> {
        let game_data = match game_data {
            mlua::Value::UserData(ud) => ud.borrow::<Arc<Mutex<GameData>>>().unwrap().clone(),
            _ => unreachable!(),
        };

        let mut game_data = game_data.lock();

        let subrows = game_data.get_switch_talk_subrows(switch_talk_id);
        // Higher subrows take precedence
        for (_, row) in subrows.iter().rev() {
            let quest0 = adjust_quest_id(row.Quest0);
            let quest1 = adjust_quest_id(row.Quest1);

            let should_check_quest0 = quest0 != 0;
            let should_check_quest1 = quest1 != 0;

            let quest0_completed = self.player_data.quest.completed.contains(quest0);
            let quest1_completed = self.player_data.quest.completed.contains(quest1);

            let quest0_passed = if should_check_quest0 {
                quest0_completed
            } else {
                true
            };
            let quest1_passed = if should_check_quest1 {
                quest1_completed
            } else {
                true
            };

            if quest0_passed && quest1_passed {
                return Some(row.DefaultTalk);
            }
        }

        None
    }

    fn register_for_content(&mut self, content_id: u16) {
        self.queued_tasks
            .push(LuaTask::RegisterForContent { content_id });
    }

    fn quest_sequence(&mut self, id: u32, sequence: u8) {
        self.queued_tasks
            .push(LuaTask::QuestSequence { id, sequence });
    }

    fn cancel_quest(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::CancelQuest { id });
    }

    fn incomplete_quest(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::IncompleteQuest { id });
    }

    fn kill(&mut self) {
        self.queued_tasks.push(LuaTask::Kill {});
    }

    fn set_online_status(&mut self, online_status_id: u8) {
        let ipc =
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::SetStatusIcon {
                    icon: OnlineStatus::from_repr(online_status_id).unwrap_or_default(),
                },
            }));

        create_ipc_self(self, ipc, self.player_data.character.actor_id);
    }

    fn abandon_content(&mut self) {
        self.queued_tasks.push(LuaTask::AbandonContent {});
    }

    fn set_item_level(&mut self, item_level: u32) {
        let ipc =
            ServerZoneIpcSegment::new(ServerZoneIpcData::ActorControlSelf(ActorControlSelf {
                category: ActorControlCategory::SetItemLevel { level: item_level },
            }));

        create_ipc_self(self, ipc, self.player_data.character.actor_id);
    }

    fn set_homepoint(&mut self, homepoint: u16) {
        self.queued_tasks.push(LuaTask::SetHomepoint { homepoint });
    }

    fn return_to_homepoint(&mut self) {
        self.queued_tasks.push(LuaTask::ReturnToHomepoint {});
    }

    fn has_aetheryte(&self, aetheryte_id: u32) -> bool {
        self.player_data.aetheryte.unlocked.contains(aetheryte_id)
    }

    fn join_content(&mut self, id: u32) {
        self.queued_tasks.push(LuaTask::JoinContent { id });
    }

    fn finish_casting_glamour(&mut self) {
        self.queued_tasks.push(LuaTask::FinishCastingGlamour {});
    }

    fn resume_event(&mut self, scene: u16, resume_id: u8, params: Vec<u32>) {
        self.queued_tasks.push(LuaTask::ResumeEvent {
            scene,
            resume_id,
            params,
        });
    }

    fn change_territory_pop_range(&mut self, territory_id: u16, pop_range_id: u32) {
        self.queued_tasks.push(LuaTask::WarpPopRange {
            territory_id,
            pop_range_id,
        });
    }

    fn remove_cooldowns(&mut self) {
        self.queued_tasks.push(LuaTask::RemoveCooldowns {});
    }

    fn toggle_howto(&mut self, value: bool, id: u32) {
        self.queued_tasks.push(LuaTask::ToggleHowTo { value, id });
    }

    fn send_mailbox_status(&mut self) {
        self.queued_tasks.push(LuaTask::SendMailboxStatus {});
    }

    fn set_grand_company(&mut self, company: GrandCompany) {
        self.queued_tasks.push(LuaTask::SetGrandCompany { company });
    }

    fn set_grand_company_rank(&mut self, rank: u8) {
        self.queued_tasks
            .push(LuaTask::SetGrandCompanyRank { rank });
    }

    fn jump(&mut self, name: String) {
        self.queued_tasks.push(LuaTask::Jump { name });
    }

    fn call(&mut self, name: String) {
        self.queued_tasks.push(LuaTask::Call { name });
    }

    fn finish_dyeing(&mut self) {
        self.queued_tasks.push(LuaTask::FinishDyeing {});
    }
}

impl UserData for LuaPlayer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut(
            "send_message",
            |lua, this, (message, param): (String, Value)| {
                let param: u8 = lua.from_value(param).unwrap_or(0);
                this.send_message(&message, param);
                Ok(())
            },
        );
        methods.add_method_mut(
            "gain_effect",
            |_, this, (effect_id, param, duration): (u16, u16, f32)| {
                this.give_status_effect(effect_id, param, duration);
                Ok(())
            },
        );
        methods.add_method_mut(
            "play_scene",
            |_, this, (scene, scene_flags, params): (u16, u32, Vec<u32>)| {
                this.play_scene(
                    scene,
                    SceneFlags::from_bits(scene_flags).unwrap_or_default(),
                    params,
                );
                Ok(())
            },
        );
        methods.add_method_mut(
            "set_position",
            |lua, this, (position, rotation): (Value, Value)| {
                let position: Position = lua.from_value(position).unwrap();
                let rotation: f32 = lua.from_value(rotation).unwrap();
                this.set_position(position, rotation);
                Ok(())
            },
        );
        methods.add_method_mut(
            "set_festival",
            |_, this, (festival1, festival2, festival3, festival4): (u32, u32, u32, u32)| {
                this.set_festival(festival1, festival2, festival3, festival4);
                Ok(())
            },
        );
        methods.add_method_mut("unlock_aetheryte", |_, this, (unlock, id): (u32, u32)| {
            this.unlock_aetheryte(unlock, id);
            Ok(())
        });
        methods.add_method_mut("unlock", |_, this, action_id: u32| {
            this.unlock(action_id);
            Ok(())
        });
        methods.add_method_mut("unlock_all", |_, this, _: ()| {
            this.unlock_all();
            Ok(())
        });
        methods.add_method_mut("set_speed", |_, this, speed: u16| {
            this.set_speed(speed);
            Ok(())
        });
        methods.add_method_mut("toggle_wireframe", |_, this, _: Value| {
            this.toggle_wireframe();
            Ok(())
        });
        methods.add_method_mut("toggle_invisibility", |_, this, _: Value| {
            this.toggle_invisiblity();
            Ok(())
        });
        methods.add_method_mut(
            "change_territory",
            |lua, this, (zone_id, exit_position, exit_rotation): (u16, Value, Value)| {
                this.change_territory(
                    zone_id,
                    lua.from_value(exit_position).unwrap_or_default(),
                    lua.from_value(exit_rotation).unwrap_or_default(),
                );
                Ok(())
            },
        );
        methods.add_method_mut("set_remake_mode", |lua, this, mode: Value| {
            let mode: RemakeMode = lua.from_value(mode).unwrap();
            this.set_remake_mode(mode);
            Ok(())
        });
        methods.add_method_mut("warp", |_, this, warp_id: u32| {
            this.warp(warp_id);
            Ok(())
        });
        methods.add_method_mut("begin_log_out", |_, this, _: ()| {
            this.begin_log_out();
            Ok(())
        });
        methods.add_method_mut("finish_event", |_, this, _: ()| {
            this.finish_event();
            Ok(())
        });
        methods.add_method_mut("unlock_classjob", |_, this, classjob_id: u8| {
            this.unlock_classjob(classjob_id);
            Ok(())
        });
        methods.add_method_mut(
            "warp_aetheryte",
            |_, this, (aetheryte_id, housing_aethernet): (u32, bool)| {
                this.warp_aetheryte(aetheryte_id, housing_aethernet);
                Ok(())
            },
        );
        methods.add_method_mut("set_level", |_, this, level: u16| {
            this.set_level(level);
            Ok(())
        });
        methods.add_method_mut("change_weather", |_, this, id: u8| {
            this.change_weather(id);
            Ok(())
        });
        methods.add_method_mut(
            "modify_currency",
            |_, this, (id, amount): (CurrencyKind, i32)| {
                this.modify_currency(id, amount, true);
                Ok(())
            },
        );
        methods.add_method_mut(
            "modify_crystals",
            |_, this, (id, amount): (CrystalKind, i32)| {
                this.modify_crystal(id, amount, true);
                Ok(())
            },
        );
        methods.add_method_mut("gm_set_orchestrion", |_, this, (value, id): (bool, u32)| {
            this.gm_set_orchestrion(value, id);
            Ok(())
        });
        methods.add_method_mut("toggle_orchestrion", |_, this, id: u32| {
            this.toggle_orchestrion(id);
            Ok(())
        });
        methods.add_method_mut("add_item", |_, this, (id, quantity): (u32, u32)| {
            // Can't think of any situations where we wouldn't want to force a client inventory update after using debug commands.
            this.add_item(id, quantity, true);
            Ok(())
        });
        methods.add_method_mut(
            "show_housing_placard",
            |_, this, (ward_index, division, plot_index): (u8, u8, u8)| {
                this.show_housing_placard(ward_index, division, plot_index);
                Ok(())
            },
        );
        methods.add_method("get_housing_ward_context", |lua, this, _: ()| {
            let context = lua.create_table()?;
            context.set(
                "territory_type_id",
                this.housing_ward_context.territory_type_id,
            )?;
            context.set("ward_index", this.housing_ward_context.ward_index)?;
            context.set("division", this.housing_ward_context.division)?;
            Ok(context)
        });
        methods.add_method(
            "get_housing_placard_location",
            |lua, this, event_arg: u32| {
                let location = housing_placard_location_from_event_arg(
                    event_arg,
                    this.housing_ward_context.division,
                );
                let table = lua.create_table()?;
                table.set("division", location.division)?;
                table.set("plot_index", location.plot_index)?;
                Ok(table)
            },
        );
        methods.add_method_mut("ensure_local_house", |_, this, _: ()| {
            this.ensure_local_house();
            Ok(())
        });
        methods.add_method_mut("ensure_local_apartment", |_, this, room_number: u16| {
            this.ensure_local_apartment(room_number);
            Ok(())
        });
        methods.add_method_mut(
            "ensure_local_house_with_options",
            |_,
             this,
             (kind, size, territory_type_id, ward_index, division, plot_index): (
                String,
                String,
                u16,
                u8,
                u8,
                u8,
            )| {
                this.ensure_local_house_with_options(
                    parse_housing_estate_kind(&kind)?,
                    parse_housing_plot_size(&size)?,
                    territory_type_id,
                    ward_index,
                    division,
                    plot_index,
                );
                Ok(())
            },
        );
        methods.add_method_mut("reset_housing", |_, this, mode: String| {
            this.reset_housing(parse_housing_reset_mode(&mode)?);
            Ok(())
        });
        methods.add_method_mut("update_housing_name", |_, this, name: String| {
            this.update_housing_name(name);
            Ok(())
        });
        methods.add_method_mut("update_housing_greeting", |_, this, greeting: String| {
            this.update_housing_greeting(greeting);
            Ok(())
        });
        methods.add_method_mut("update_housing_light", |_, this, level: u8| {
            this.update_housing_light(level);
            Ok(())
        });
        methods.add_method_mut(
            "update_housing_exterior",
            |_, this, (field, value): (String, u16)| {
                this.update_housing_exterior(parse_housing_exterior_field(&field)?, value);
                Ok(())
            },
        );
        methods.add_method_mut(
            "update_housing_exterior_color",
            |_, this, (field, value): (String, u8)| {
                this.update_housing_exterior_color(
                    parse_housing_exterior_color_field(&field)?,
                    value,
                );
                Ok(())
            },
        );
        methods.add_method_mut(
            "update_housing_interior",
            |_, this, (field, value): (String, u32)| {
                let field = parse_housing_interior_field(&field)?;
                this.update_housing_interior(field, validate_housing_interior_value(field, value)?);
                Ok(())
            },
        );
        methods.add_method_mut(
            "apply_housing_preset",
            |_, this, (path, scope, reload): (String, String, bool)| {
                this.apply_housing_preset(path, parse_housing_preset_scope(&scope)?, reload);
                Ok(())
            },
        );
        methods.add_method_mut(
            "apply_latest_housing_preset",
            |_, this, (scope, reload): (String, bool)| {
                this.apply_latest_housing_preset(parse_housing_preset_scope(&scope)?, reload);
                Ok(())
            },
        );
        methods.add_method_mut("repeat_housing_preset", |_, this, reload: bool| {
            this.repeat_housing_preset(reload);
            Ok(())
        });
        methods.add_method_mut(
            "check_housing_preset",
            |_, this, (path, scope): (String, String)| {
                this.check_housing_preset(path, parse_housing_preset_scope(&scope)?);
                Ok(())
            },
        );
        methods.add_method_mut("check_latest_housing_preset", |_, this, scope: String| {
            this.check_latest_housing_preset(parse_housing_preset_scope(&scope)?);
            Ok(())
        });
        methods.add_method_mut("check_repeated_housing_preset", |_, this, _: ()| {
            this.check_repeated_housing_preset();
            Ok(())
        });
        methods.add_method_mut("give_housing_kit", |_, this, kit: String| {
            this.give_housing_kit(parse_housing_kit(&kit)?);
            Ok(())
        });
        methods.add_method_mut("enter_local_house", |_, this, _: ()| {
            this.enter_local_house();
            Ok(())
        });
        methods.add_method_mut("enter_local_apartment", |_, this, room_number: u16| {
            this.enter_local_apartment(room_number);
            Ok(())
        });
        methods.add_method_mut("exit_local_house", |_, this, _: ()| {
            this.exit_local_house();
            Ok(())
        });
        methods.add_method_mut("reload_housing", |_, this, _: ()| {
            this.reload_housing();
            Ok(())
        });
        methods.add_method_mut("unlock_content", |_, this, id: u16| {
            this.unlock_content(id);
            Ok(())
        });
        methods.add_method_mut("unlock_all_content", |_, this, _: ()| {
            this.unlock_all_content();
            Ok(())
        });
        methods.add_method_mut("add_exp", |_, this, amount: i32| {
            this.add_exp(amount);
            Ok(())
        });
        methods.add_method_mut(
            "start_event",
            |_, this, (event_id, event_type, event_arg): (u32, u8, u32)| {
                this.start_event(
                    event_id,
                    EventType::from_repr(event_type).unwrap(),
                    event_arg,
                );
                Ok(())
            },
        );
        methods.add_method_mut("set_inn_wakeup", |_, this, watched: bool| {
            this.set_inn_wakeup(watched);
            Ok(())
        });
        methods.add_method_mut(
            "do_solnine_teleporter",
            |_,
             this,
             (event_id, path_id, unk2, unk3, speed, unk4, unk5): (
                u32,
                u32,
                u16,
                u16,
                u16,
                u16,
                u32,
            )| {
                this.do_solnine_teleporter(event_id, path_id, unk2, unk3, speed, unk4, unk5);
                Ok(())
            },
        );
        methods.add_method_mut("toggle_mount", |_, this, id: u32| {
            this.toggle_mount(id);
            Ok(())
        });
        methods.add_method_mut("toggle_glasses_style", |_, this, id: u32| {
            this.toggle_glasses_style(id);
            Ok(())
        });
        methods.add_method_mut("toggle_glasses_style_all", |_, this, _: ()| {
            this.toggle_glasses_style_all();
            Ok(())
        });
        methods.add_method_mut("toggle_ornament", |_, this, id: u32| {
            this.toggle_ornament(id);
            Ok(())
        });
        methods.add_method_mut("toggle_ornament_all", |_, this, _: ()| {
            this.toggle_ornament_all();
            Ok(())
        });
        methods.add_method_mut("unlock_buddy_equip", |_, this, id: u32| {
            this.unlock_buddy_equip(id);
            Ok(())
        });
        methods.add_method_mut("unlock_buddy_equip_all", |_, this, _: ()| {
            this.unlock_buddy_equip_all();
            Ok(())
        });
        methods.add_method_mut("toggle_chocobo_taxi_stand", |_, this, id: u32| {
            this.toggle_chocobo_taxi_stand(id);
            Ok(())
        });
        methods.add_method_mut("toggle_chocobo_taxi_stand_all", |_, this, _: ()| {
            this.toggle_chocobo_taxi_stand_all();
            Ok(())
        });
        methods.add_method_mut("toggle_caught_fish", |_, this, id: u32| {
            this.toggle_caught_fish(id);
            Ok(())
        });
        methods.add_method_mut("toggle_caught_fish_all", |_, this, _: ()| {
            this.toggle_caught_fish_all();
            Ok(())
        });
        methods.add_method_mut("toggle_caught_spearfish", |_, this, id: u32| {
            this.toggle_caught_spearfish(id);
            Ok(())
        });
        methods.add_method_mut("toggle_caught_spearfish_all", |_, this, _: ()| {
            this.toggle_caught_spearfish_all();
            Ok(())
        });
        methods.add_method_mut("toggle_triple_triad_card", |_, this, id: u32| {
            this.toggle_triple_triad_card(id);
            Ok(())
        });
        methods.add_method_mut("toggle_triple_triad_card_all", |_, this, _: ()| {
            this.toggle_triple_triad_card_all();
            Ok(())
        });
        methods.add_method_mut("toggle_adventure", |_, this, id: u32| {
            this.toggle_adventure(id);
            Ok(())
        });
        methods.add_method_mut("toggle_adventure_all", |_, this, _: ()| {
            this.toggle_adventure_all();
            Ok(())
        });
        methods.add_method_mut(
            "toggle_cutscene_seen",
            |_, this, (id, value): (u32, bool)| {
                this.toggle_cutscene_seen(id, value);
                Ok(())
            },
        );
        methods.add_method_mut("toggle_cutscene_seen_all", |_, this, _: ()| {
            this.toggle_cutscene_seen_all();
            Ok(())
        });
        methods.add_method_mut("toggle_minion", |_, this, id: u32| {
            this.toggle_minion(id);
            Ok(())
        });
        methods.add_method_mut("toggle_minion_all", |_, this, _: ()| {
            this.toggle_minion_all();
            Ok(())
        });
        methods.add_method_mut("toggle_aether_current", |_, this, id: u32| {
            this.toggle_aether_current(id);
            Ok(())
        });
        methods.add_method_mut("toggle_aether_current_all", |_, this, _: ()| {
            this.toggle_aether_current_all();
            Ok(())
        });
        methods.add_method_mut("toggle_aether_current_comp_flg_set", |_, this, id: u32| {
            this.toggle_aether_current_comp_flg_set(id);
            Ok(())
        });
        methods.add_method_mut(
            "toggle_aether_current_comp_flg_set_all",
            |_, this, _: ()| {
                this.toggle_aether_current_comp_flg_set_all();
                Ok(())
            },
        );
        methods.add_method_mut(
            "move_to_pop_range",
            |lua, this, (id, fade_out): (u32, Value)| {
                let fade_out: bool = lua.from_value(fade_out).unwrap_or_default();
                this.move_to_pop_range(id, fade_out);
                Ok(())
            },
        );
        methods.add_method_mut("set_hp", |_, this, hp: u32| {
            this.set_hp(hp);
            Ok(())
        });
        methods.add_method_mut("set_mp", |_, this, mp: u16| {
            this.set_mp(mp);
            Ok(())
        });
        methods.add_method_mut("set_race", |_, this, race: u8| {
            this.set_race(race);
            Ok(())
        });
        methods.add_method_mut("set_tribe", |_, this, tribe: u8| {
            this.set_tribe(tribe);
            Ok(())
        });
        methods.add_method_mut("set_sex", |_, this, sex: u8| {
            this.set_sex(sex);
            Ok(())
        });
        methods.add_method("get_effect", |_, this, effect_id: u16| {
            Ok(this.status_effects.get(effect_id))
        });
        methods.add_method_mut("start_talk_event", |_, this, _: ()| {
            this.start_talk_event();
            Ok(())
        });
        methods.add_method_mut("accept_quest", |_, this, quest_id: u32| {
            this.accept_quest(quest_id);
            Ok(())
        });
        methods.add_method_mut("finish_quest", |_, this, quest_id: u32| {
            this.finish_quest(quest_id);
            Ok(())
        });
        methods.add_method_mut("has_quest", |_, this, quest_id: u16| {
            Ok(this
                .player_data
                .quest
                .active
                .0
                .iter()
                .any(|quest| quest.id == quest_id)
                || this.player_data.quest.completed.contains(quest_id as u32))
        });
        methods.add_method_mut("has_seen_cutscene", |_, this, cutscene_id: u32| {
            Ok(this.player_data.unlock.cutscene_seen.contains(cutscene_id))
        });
        methods.add_method_mut("commence_duty", |_, this, director_id: u32| {
            this.commence_duty(director_id);
            Ok(())
        });
        methods.add_method_mut(
            "get_switch_talk_target",
            |lua, this, switch_talk_target: u32| {
                Ok(this.get_switch_talk_target(
                    lua.globals().get("GAME_DATA").unwrap(),
                    switch_talk_target,
                ))
            },
        );
        methods.add_method_mut("register_for_content", |_, this, content_id: u16| {
            this.register_for_content(content_id);
            Ok(())
        });
        methods.add_method_mut(
            "quest_sequence",
            |_, this, (quest_id, sequence): (u32, u8)| {
                this.quest_sequence(quest_id, sequence);
                Ok(())
            },
        );
        methods.add_method_mut("cancel_quest", |_, this, quest_id: u32| {
            this.cancel_quest(quest_id);
            Ok(())
        });
        methods.add_method_mut("incomplete_quest", |_, this, quest_id: u32| {
            this.incomplete_quest(quest_id);
            Ok(())
        });
        methods.add_method_mut("kill", |_, this, _: ()| {
            this.kill();
            Ok(())
        });
        methods.add_method_mut("set_online_status", |_, this, online_status_id: u8| {
            this.set_online_status(online_status_id);
            Ok(())
        });
        methods.add_method_mut("abandon_content", |_, this, _: ()| {
            this.abandon_content();
            Ok(())
        });
        methods.add_method_mut("set_item_level", |_, this, item_level: u32| {
            this.set_item_level(item_level);
            Ok(())
        });
        methods.add_method_mut("set_homepoint", |_, this, homepoint: u16| {
            this.set_homepoint(homepoint);
            Ok(())
        });
        methods.add_method_mut("return_to_homepoint", |_, this, _: ()| {
            this.return_to_homepoint();
            Ok(())
        });
        methods.add_method("has_aetheryte", |_, this, aetheryte_id: u32| {
            Ok(this.has_aetheryte(aetheryte_id))
        });
        methods.add_method_mut("join_content", |_, this, id: u32| {
            this.join_content(id);
            Ok(())
        });
        methods.add_method_mut("finish_casting_glamour", |_, this, _: ()| {
            this.finish_casting_glamour();
            Ok(())
        });
        methods.add_method_mut(
            "resume_event",
            |_, this, (scene, resume_id, params): (u16, u8, Vec<u32>)| {
                this.resume_event(scene, resume_id, params);
                Ok(())
            },
        );
        methods.add_method_mut(
            "change_territory_pop_range",
            |_, this, (territory_id, pop_range_id): (u16, u32)| {
                this.change_territory_pop_range(territory_id, pop_range_id);
                Ok(())
            },
        );
        methods.add_method_mut("remove_cooldowns", |_, this, _: ()| {
            this.remove_cooldowns();
            Ok(())
        });
        methods.add_method_mut("toggle_howto", |_, this, (value, id): (bool, u32)| {
            this.toggle_howto(value, id);
            Ok(())
        });
        methods.add_method_mut("send_mailbox_status", |_, this, _: ()| {
            this.send_mailbox_status();
            Ok(())
        });
        methods.add_method_mut("set_grand_company", |_, this, company: u8| {
            this.set_grand_company(GrandCompany::from_repr(company as usize).unwrap_or_default());
            Ok(())
        });
        methods.add_method_mut("set_grand_company_rank", |_, this, rank: u8| {
            this.set_grand_company_rank(rank);
            Ok(())
        });
        methods.add_method_mut("jump", |_, this, name: String| {
            this.jump(name);
            Ok(())
        });
        methods.add_method_mut("call", |_, this, name: String| {
            this.call(name);
            Ok(())
        });
        methods.add_method_mut("finish_dyeing", |_, this, _: ()| {
            this.finish_dyeing();
            Ok(())
        });
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| {
            Ok(ObjectTypeId {
                object_id: this.player_data.character.actor_id,
                object_type: ObjectTypeKind::None,
            })
        });

        fields.add_field_method_get("teleport_query", |_, this| {
            Ok(this.player_data.teleport_query.clone())
        });
        fields.add_field_method_get("rotation", |_, this| Ok(this.player_data.volatile.rotation));
        fields.add_field_method_get("position", |_, this| Ok(this.player_data.volatile.position));
        fields.add_field_method_get("inventory", |_, this| {
            Ok(this.player_data.inventory.clone())
        });
        fields.add_field_method_get("zone", |_, this| Ok(this.zone_data.clone()));
        // Helper method to reduce the amount of typing for gil
        fields.add_field_method_get("gil", |_, this| {
            Ok(this.player_data.inventory.currency.gil.quantity)
        });
        fields.add_field_method_get("saw_inn_wakeup", |_, this| {
            Ok(this.player_data.saw_inn_wakeup)
        });
        fields.add_field_method_get("content", |_, this| Ok(this.content_data));
        fields.add_field_method_get("parameters", |_, this| Ok(this.base_parameters.clone()));
        fields.add_field_method_get("rested_exp", |_, this| {
            Ok(this.player_data.classjob.rested_exp)
        });
        fields.add_field_method_get("active_grand_company", |_, this| {
            Ok(this.player_data.grand_company.active_company)
        });
        fields.add_field_method_get("city_state", |_, this| Ok(this.player_data.city_state));
    }
}

fn parse_housing_estate_kind(value: &str) -> mlua::Result<HousingEstateKind> {
    match value.to_ascii_lowercase().as_str() {
        "personal" => Ok(HousingEstateKind::Personal),
        "fc" | "freecompany" | "free_company" => Ok(HousingEstateKind::FreeCompany),
        _ => Err(mlua::Error::external(format!(
            "invalid housing estate kind: {value}"
        ))),
    }
}

fn parse_housing_plot_size(value: &str) -> mlua::Result<PlotSize> {
    match value.to_ascii_lowercase().as_str() {
        "small" => Ok(PlotSize::Small),
        "medium" => Ok(PlotSize::Medium),
        "large" => Ok(PlotSize::Large),
        _ => Err(mlua::Error::external(format!(
            "invalid housing plot size: {value}"
        ))),
    }
}

fn parse_housing_reset_mode(value: &str) -> mlua::Result<HousingResetMode> {
    match value.to_ascii_lowercase().as_str() {
        "furniture" => Ok(HousingResetMode::Furniture),
        "estate" => Ok(HousingResetMode::Estate),
        "all" => Ok(HousingResetMode::All),
        _ => Err(mlua::Error::external(format!(
            "invalid housing reset mode: {value}"
        ))),
    }
}

fn parse_housing_kit(value: &str) -> mlua::Result<HousingKit> {
    match value.to_ascii_lowercase().as_str() {
        "indoor" => Ok(HousingKit::Indoor),
        "outdoor" => Ok(HousingKit::Outdoor),
        "npc" => Ok(HousingKit::Npc),
        _ => Err(mlua::Error::external(format!(
            "invalid housing kit: {value}"
        ))),
    }
}

fn parse_housing_preset_scope(value: &str) -> mlua::Result<HousingPresetScope> {
    match value.to_ascii_lowercase().as_str() {
        "" | "all" => Ok(HousingPresetScope::All),
        "interior" | "indoor" => Ok(HousingPresetScope::Interior),
        "exterior" | "outdoor" => Ok(HousingPresetScope::Exterior),
        _ => Err(mlua::Error::external(format!(
            "invalid housing preset scope: {value}"
        ))),
    }
}

fn parse_housing_exterior_field(value: &str) -> mlua::Result<HousingExteriorField> {
    match value.to_ascii_lowercase().as_str() {
        "roof" => Ok(HousingExteriorField::Roof),
        "walls" => Ok(HousingExteriorField::Walls),
        "windows" => Ok(HousingExteriorField::Windows),
        "door" => Ok(HousingExteriorField::Door),
        "roof_fixture" => Ok(HousingExteriorField::RoofFixture),
        "wall_fixture" => Ok(HousingExteriorField::WallFixture),
        "above_door_banner" => Ok(HousingExteriorField::AboveDoorBanner),
        "fence" => Ok(HousingExteriorField::Fence),
        _ => Err(mlua::Error::external(format!(
            "invalid housing exterior field: {value}"
        ))),
    }
}

fn parse_housing_exterior_color_field(value: &str) -> mlua::Result<HousingExteriorColorField> {
    match value.to_ascii_lowercase().as_str() {
        "roof" => Ok(HousingExteriorColorField::Roof),
        "walls" => Ok(HousingExteriorColorField::Walls),
        "windows" => Ok(HousingExteriorColorField::Windows),
        "door" => Ok(HousingExteriorColorField::Door),
        "roof_fixture" => Ok(HousingExteriorColorField::RoofFixture),
        "wall_fixture" => Ok(HousingExteriorColorField::WallFixture),
        "above_door_banner" => Ok(HousingExteriorColorField::AboveDoorBanner),
        "fence" => Ok(HousingExteriorColorField::Fence),
        _ => Err(mlua::Error::external(format!(
            "invalid housing exterior color field: {value}"
        ))),
    }
}

fn parse_housing_interior_field(value: &str) -> mlua::Result<HousingInteriorField> {
    match value.to_ascii_lowercase().as_str() {
        "window_style" => Ok(HousingInteriorField::WindowStyle),
        "door_style" => Ok(HousingInteriorField::DoorStyle),
        "door_stain" => Ok(HousingInteriorField::DoorStain),
        "ground_walls" => Ok(HousingInteriorField::GroundWalls),
        "ground_floor" => Ok(HousingInteriorField::GroundFloor),
        "ground_chandelier" => Ok(HousingInteriorField::GroundChandelier),
        "top_walls" => Ok(HousingInteriorField::TopWalls),
        "top_floor" => Ok(HousingInteriorField::TopFloor),
        "top_chandelier" => Ok(HousingInteriorField::TopChandelier),
        "cellar_walls" => Ok(HousingInteriorField::CellarWalls),
        "cellar_floor" => Ok(HousingInteriorField::CellarFloor),
        "cellar_chandelier" => Ok(HousingInteriorField::CellarChandelier),
        _ => Err(mlua::Error::external(format!(
            "invalid housing interior field: {value}"
        ))),
    }
}

fn validate_housing_interior_value(field: HousingInteriorField, value: u32) -> mlua::Result<u32> {
    if matches!(
        field,
        HousingInteriorField::WindowStyle
            | HousingInteriorField::DoorStyle
            | HousingInteriorField::DoorStain
    ) && value > u16::MAX as u32
    {
        return Err(mlua::Error::external(format!(
            "value {value} is out of range for {field:?}"
        )));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kawari::ipc::zone::PlotSize;
    use mlua::Function;
    use parking_lot::Mutex;

    use super::*;
    use crate::lua::{
        HousingEstateKind, HousingExteriorColorField, HousingExteriorField, HousingInteriorField,
        HousingKit, HousingPresetScope, HousingResetMode,
    };

    fn load_housing_lua_with_messages() -> (mlua::Lua, Arc<Mutex<Vec<String>>>) {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();

        let messages = Arc::new(Mutex::new(Vec::new()));
        let captured_messages = messages.clone();
        lua.globals()
            .set(
                "printf",
                lua.create_function(move |_, values: mlua::MultiValue| {
                    if let Some(mlua::Value::String(message)) = values.into_iter().nth(1) {
                        captured_messages.lock().push(message.to_str()?.to_string());
                    }

                    Ok(())
                })
                .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        (lua, messages)
    }

    #[test]
    fn housing_lua_commands_registry_points_housing_at_debug_script() {
        let commands = include_str!("../../../../resources/scripts/commands/Commands.lua");
        let legacy_housing_target = ["GM_DIR", "..", r#""Housing.lua""#].concat();
        let housing_registers = commands
            .lines()
            .filter(|line| line.contains(r#"registerCommand("housing""#))
            .collect::<Vec<_>>();

        assert!(
            housing_registers
                .iter()
                .any(|line| line.contains(r#"DBG_DIR.."Housing.lua""#))
        );
        assert!(
            !housing_registers
                .iter()
                .any(|line| line.contains(&legacy_housing_target))
        );
    }

    #[test]
    fn housing_lua_invalid_subcommand_prints_usage_and_queues_no_tasks() {
        let (lua, messages) = load_housing_lua_with_messages();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "unknown").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        assert!(player.queued_tasks.is_empty());

        let messages = messages.lock();
        let usage = messages.join("\n");
        assert!(usage.contains("Usage: !housing"));
        assert!(usage.contains("reference_medium_interior"));
        assert!(usage.contains("all|interior|indoor|exterior|outdoor"));
    }

    #[test]
    fn housing_lua_enter_typo_prints_usage_and_queues_no_tasks() {
        let (lua, messages) = load_housing_lua_with_messages();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "enter").unwrap();
        args.set(2, "typo").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        assert!(player.queued_tasks.is_empty());

        let usage = messages.lock().join("\n");
        assert!(usage.contains("Usage: !housing"));
        assert!(usage.contains("!housing enter"));
    }

    #[test]
    fn housing_lua_bare_enter_queues_local_house_enter() {
        let (lua, _messages) = load_housing_lua_with_messages();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "enter").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [LuaTask::EnterLocalHouse {}] => {}
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_enter_house_queues_local_house_enter() {
        let (lua, _messages) = load_housing_lua_with_messages();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "enter").unwrap();
        args.set(2, "house").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [LuaTask::EnterLocalHouse {}] => {}
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_enter_house_extra_prints_usage_and_queues_no_tasks() {
        let (lua, messages) = load_housing_lua_with_messages();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "enter").unwrap();
        args.set(2, "house").unwrap();
        args.set(3, "extra").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        assert!(player.queued_tasks.is_empty());

        let usage = messages.lock().join("\n");
        assert!(usage.contains("Usage: !housing"));
        assert!(usage.contains("!housing enter"));
    }

    #[test]
    fn housing_lua_no_arg_command_queues_default_local_house() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [
                LuaTask::EnsureLocalHouseWithOptions {
                    kind,
                    size,
                    territory_type_id,
                    ward_index,
                    division,
                    plot_index,
                },
            ] => {
                assert_eq!(*kind, HousingEstateKind::Personal);
                assert_eq!(*size, PlotSize::Large);
                assert_eq!(*territory_type_id, 340);
                assert_eq!(*ward_index, 0);
                assert_eq!(*division, 0);
                assert_eq!(*plot_index, 5);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_testhouse_command_queues_zero_based_options() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "testhouse").unwrap();
        args.set(2, "fc").unwrap();
        args.set(3, "medium").unwrap();
        args.set(4, "341").unwrap();
        args.set(5, "3").unwrap();
        args.set(6, "13").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [
                LuaTask::EnsureLocalHouseWithOptions {
                    kind,
                    size,
                    territory_type_id,
                    ward_index,
                    division,
                    plot_index,
                },
            ] => {
                assert_eq!(*kind, HousingEstateKind::FreeCompany);
                assert_eq!(*size, PlotSize::Medium);
                assert_eq!(*territory_type_id, 341);
                assert_eq!(*ward_index, 2);
                assert_eq!(*division, 0);
                assert_eq!(*plot_index, 12);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_preset_command_queues_scope_and_path_with_spaces() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "preset").unwrap();
        args.set(2, "interior").unwrap();
        args.set(3, "CAFE").unwrap();
        args.set(4, "CAT").unwrap();
        args.set(5, "WALK").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [
                LuaTask::ApplyHousingPreset {
                    path,
                    scope,
                    reload,
                },
            ] => {
                assert_eq!(path, "CAFE CAT WALK");
                assert_eq!(*scope, HousingPresetScope::Interior);
                assert!(!reload);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_preset_latest_queues_scope_and_reload() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "preset").unwrap();
        args.set(2, "latest").unwrap();
        args.set(3, "interior").unwrap();
        args.set(4, "--reload").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [LuaTask::ApplyLatestHousingPreset { scope, reload }] => {
                assert_eq!(*scope, HousingPresetScope::Interior);
                assert!(*reload);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_preset_repeat_queues_reload() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "preset").unwrap();
        args.set(2, "repeat").unwrap();
        args.set(3, "--reload").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [LuaTask::RepeatHousingPreset { reload }] => assert!(*reload),
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_preset_path_can_contain_reload_token() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "preset").unwrap();
        args.set(2, "interior").unwrap();
        args.set(3, "reload").unwrap();
        args.set(4, "room").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [
                LuaTask::ApplyHousingPreset {
                    path,
                    scope,
                    reload,
                },
            ] => {
                assert_eq!(path, "reload room");
                assert_eq!(*scope, HousingPresetScope::Interior);
                assert!(!reload);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_preset_check_rejects_reload_flag() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "preset").unwrap();
        args.set(2, "check").unwrap();
        args.set(3, "latest").unwrap();
        args.set(4, "--reload").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        assert!(player.queued_tasks.is_empty());
    }

    #[test]
    fn housing_lua_preset_repeat_rejects_explicit_scope() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "preset").unwrap();
        args.set(2, "check").unwrap();
        args.set(3, "exterior").unwrap();
        args.set(4, "repeat").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        assert!(player.queued_tasks.is_empty());
    }

    #[test]
    fn housing_lua_preset_check_latest_queues_check() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "preset").unwrap();
        args.set(2, "check").unwrap();
        args.set(3, "exterior").unwrap();
        args.set(4, "latest").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [LuaTask::CheckLatestHousingPreset { scope }] => {
                assert_eq!(*scope, HousingPresetScope::Exterior);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_reload_queues_housing_reload() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let args = lua.create_table().unwrap();
        args.set(1, "reload").unwrap();

        let on_command: Function = lua.globals().get("onCommand").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [LuaTask::ReloadHousing {}] => {}
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn ensure_local_house_with_options_queues_parameterized_task() {
        let mut player = LuaPlayer::default();

        player.ensure_local_house_with_options(
            HousingEstateKind::FreeCompany,
            PlotSize::Medium,
            341,
            2,
            1,
            12,
        );

        match player.queued_tasks.as_slice() {
            [
                LuaTask::EnsureLocalHouseWithOptions {
                    kind,
                    size,
                    territory_type_id,
                    ward_index,
                    division,
                    plot_index,
                },
            ] => {
                assert_eq!(*kind, HousingEstateKind::FreeCompany);
                assert_eq!(*size, PlotSize::Medium);
                assert_eq!(*territory_type_id, 341);
                assert_eq!(*ward_index, 2);
                assert_eq!(*division, 1);
                assert_eq!(*plot_index, 12);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn apartment_methods_queue_room_tasks() {
        let mut player = LuaPlayer::default();

        player.ensure_local_apartment(1);
        player.enter_local_apartment(1);

        match player.queued_tasks.as_slice() {
            [
                LuaTask::EnsureLocalApartment { room_number },
                LuaTask::EnterLocalApartment {
                    room_number: enter_room_number,
                },
            ] => {
                assert_eq!(*room_number, 1);
                assert_eq!(*enter_room_number, 1);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_reset_name_greeting_light_and_kit_queue_tasks() {
        let mut player = LuaPlayer::default();

        player.reset_housing(HousingResetMode::Furniture);
        player.update_housing_name("My Estate".to_string());
        player.update_housing_greeting("Welcome home".to_string());
        player.update_housing_light(3);
        player.give_housing_kit(HousingKit::Indoor);

        assert!(matches!(
            player.queued_tasks[0],
            LuaTask::ResetHousing {
                mode: HousingResetMode::Furniture
            }
        ));
        assert!(matches!(
            &player.queued_tasks[1],
            LuaTask::UpdateHousingName { name } if name == "My Estate"
        ));
        assert!(matches!(
            &player.queued_tasks[2],
            LuaTask::UpdateHousingGreeting { greeting } if greeting == "Welcome home"
        ));
        assert!(matches!(
            player.queued_tasks[3],
            LuaTask::UpdateHousingLight { level: 3 }
        ));
        assert!(matches!(
            player.queued_tasks[4],
            LuaTask::GiveHousingKit {
                kit: HousingKit::Indoor
            }
        ));
    }

    #[test]
    fn housing_fixture_methods_queue_exterior_and_interior_tasks() {
        let mut player = LuaPlayer::default();

        player.update_housing_exterior(HousingExteriorField::Roof, 12);
        player.update_housing_exterior_color(HousingExteriorColorField::Walls, 5);
        player.update_housing_interior(HousingInteriorField::GroundFloor, 65591);

        assert!(matches!(
            player.queued_tasks[0],
            LuaTask::UpdateHousingExterior {
                field: HousingExteriorField::Roof,
                value: 12
            }
        ));
        assert!(matches!(
            player.queued_tasks[1],
            LuaTask::UpdateHousingExteriorColor {
                field: HousingExteriorColorField::Walls,
                value: 5
            }
        ));
        assert!(matches!(
            player.queued_tasks[2],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::GroundFloor,
                value: 65591
            }
        ));
    }

    #[test]
    fn housing_lua_fixture_commands_queue_expected_tasks() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let on_command: Function = lua.globals().get("onCommand").unwrap();

        let exterior_args = lua.create_table().unwrap();
        exterior_args.set(1, "exterior").unwrap();
        exterior_args.set(2, "color").unwrap();
        exterior_args.set(3, "walls").unwrap();
        exterior_args.set(4, "5").unwrap();
        on_command
            .call::<()>((player.clone(), exterior_args, "housing"))
            .unwrap();

        let interior_args = lua.create_table().unwrap();
        interior_args.set(1, "interior").unwrap();
        interior_args.set(2, "ground_floor").unwrap();
        interior_args.set(3, "65591").unwrap();
        on_command
            .call::<()>((player.clone(), interior_args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        assert!(matches!(
            player.queued_tasks[0],
            LuaTask::UpdateHousingExteriorColor {
                field: HousingExteriorColorField::Walls,
                value: 5
            }
        ));
        assert!(matches!(
            player.queued_tasks[1],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::GroundFloor,
                value: 65591
            }
        ));
    }

    #[test]
    fn housing_lua_interior_preset_queues_fixture_tasks() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let on_command: Function = lua.globals().get("onCommand").unwrap();

        let args = lua.create_table().unwrap();
        args.set(1, "interior").unwrap();
        args.set(2, "preset").unwrap();
        args.set(3, "reference_medium_interior").unwrap();
        on_command
            .call::<()>((player.clone(), args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        let tasks = &player.queued_tasks;
        assert_eq!(tasks.len(), 12);
        assert!(matches!(
            tasks[0],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::WindowStyle,
                value: 2601
            }
        ));
        assert!(matches!(
            tasks[1],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::DoorStyle,
                value: 553
            }
        ));
        assert!(matches!(
            tasks[2],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::DoorStain,
                value: 365
            }
        ));
        assert!(matches!(
            tasks[5],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::GroundChandelier,
                value: 65821
            }
        ));
        assert!(matches!(
            tasks[8],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::TopChandelier,
                value: 65848
            }
        ));
        assert!(matches!(
            tasks[11],
            LuaTask::UpdateHousingInterior {
                field: HousingInteriorField::CellarChandelier,
                value: 65796
            }
        ));
    }

    #[test]
    fn housing_lua_apartment_commands_queue_room_one() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let on_command: Function = lua.globals().get("onCommand").unwrap();

        let apartment_args = lua.create_table().unwrap();
        apartment_args.set(1, "apartment").unwrap();
        apartment_args.set(2, "1").unwrap();
        on_command
            .call::<()>((player.clone(), apartment_args, "housing"))
            .unwrap();

        let enter_apartment_args = lua.create_table().unwrap();
        enter_apartment_args.set(1, "enter").unwrap();
        enter_apartment_args.set(2, "apartment").unwrap();
        enter_apartment_args.set(3, "1").unwrap();
        on_command
            .call::<()>((player.clone(), enter_apartment_args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        match player.queued_tasks.as_slice() {
            [
                LuaTask::EnsureLocalApartment { room_number },
                LuaTask::EnterLocalApartment {
                    room_number: enter_room_number,
                },
            ] => {
                assert_eq!(*room_number, 1);
                assert_eq!(*enter_room_number, 1);
            }
            other => panic!("unexpected tasks: {other:?}"),
        }
    }

    #[test]
    fn housing_lua_apartment_commands_reject_room_numbers_above_packed_limit() {
        let lua = mlua::Lua::new();
        lua.globals().set("GM_RANK_DEBUG", 1).unwrap();
        lua.globals()
            .set(
                "printf",
                lua.create_function(|_, _: mlua::MultiValue| Ok(()))
                    .unwrap(),
            )
            .unwrap();
        lua.load(include_str!(
            "../../../../resources/scripts/commands/debug/Housing.lua"
        ))
        .exec()
        .unwrap();

        let player = lua.create_userdata(LuaPlayer::default()).unwrap();
        let on_command: Function = lua.globals().get("onCommand").unwrap();

        let apartment_args = lua.create_table().unwrap();
        apartment_args.set(1, "apartment").unwrap();
        apartment_args.set(2, "1024").unwrap();
        on_command
            .call::<()>((player.clone(), apartment_args, "housing"))
            .unwrap();

        let enter_apartment_args = lua.create_table().unwrap();
        enter_apartment_args.set(1, "enter").unwrap();
        enter_apartment_args.set(2, "apartment").unwrap();
        enter_apartment_args.set(3, "1024").unwrap();
        on_command
            .call::<()>((player.clone(), enter_apartment_args, "housing"))
            .unwrap();

        let player = player.borrow::<LuaPlayer>().unwrap();
        assert!(
            player.queued_tasks.is_empty(),
            "Lua boundary must reject apartment rooms above the packed HouseId limit"
        );
    }
}
