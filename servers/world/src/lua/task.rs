use crate::{
    RemakeMode,
    inventory::{CrystalKind, CurrencyKind},
};
use kawari::{
    common::Position,
    ipc::zone::{EventType, GrandCompany, PlotSize, SceneFlags, ServerZoneIpcSegment},
    packet::PacketSegment,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HousingEstateKind {
    Personal,
    FreeCompany,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HousingResetMode {
    Furniture,
    Estate,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HousingPresetScope {
    All,
    Interior,
    Exterior,
}

impl HousingPresetScope {
    pub fn includes_interior(self) -> bool {
        matches!(self, Self::All | Self::Interior)
    }

    pub fn includes_exterior(self) -> bool {
        matches!(self, Self::All | Self::Exterior)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HousingKit {
    Indoor,
    Outdoor,
    Npc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HousingExteriorField {
    Roof,
    Walls,
    Windows,
    Door,
    RoofFixture,
    WallFixture,
    AboveDoorBanner,
    Fence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HousingExteriorColorField {
    Roof,
    Walls,
    Windows,
    Door,
    RoofFixture,
    WallFixture,
    AboveDoorBanner,
    Fence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HousingInteriorField {
    WindowStyle,
    DoorStyle,
    DoorStain,
    GroundWalls,
    GroundFloor,
    GroundChandelier,
    TopWalls,
    TopFloor,
    TopChandelier,
    CellarWalls,
    CellarFloor,
    CellarChandelier,
}

#[derive(Clone, Debug)]
pub enum LuaTask {
    ChangeTerritory {
        zone_id: u16,
        exit_position: Option<Position>,
        exit_rotation: Option<f32>,
    },
    SetRemakeMode(RemakeMode),
    Warp {
        warp_id: u32,
    },
    BeginLogOut,
    FinishEvent {},
    UnlockClassJob {
        classjob_id: u8,
    },
    WarpAetheryte {
        aetheryte_id: u32,
        housing_aethernet: bool,
    },
    ToggleInvisibility {
        invisible: bool,
    },
    Unlock {
        id: u32,
    },
    UnlockAll {},
    UnlockAetheryte {
        id: u32,
        on: bool,
    },
    SetLevel {
        level: u16,
    },
    ChangeWeather {
        id: u8,
    },
    ModifyCurrency {
        id: CurrencyKind,
        amount: i32,
        send_client_update: bool,
    },
    ModifyCrystal {
        id: CrystalKind,
        amount: i32,
        send_client_update: bool,
    },
    GmSetOrchestrion {
        value: bool,
        id: u32,
    },
    ToggleOrchestrion {
        id: u32,
    },
    AddItem {
        id: u32,
        quantity: u32,
        send_client_update: bool,
    },
    ShowHousingPlacard {
        ward_index: u8,
        division: u8,
        plot_index: u8,
    },
    EnsureTestApartment {
        room_number: u16,
    },
    EnsureTestHouse {},
    EnsureTestHouseWithOptions {
        kind: HousingEstateKind,
        size: PlotSize,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: u8,
    },
    ResetHousing {
        mode: HousingResetMode,
    },
    UpdateHousingName {
        name: String,
    },
    UpdateHousingGreeting {
        greeting: String,
    },
    UpdateHousingLight {
        level: u8,
    },
    UpdateHousingExterior {
        field: HousingExteriorField,
        value: u16,
    },
    UpdateHousingExteriorColor {
        field: HousingExteriorColorField,
        value: u8,
    },
    UpdateHousingInterior {
        field: HousingInteriorField,
        value: u32,
    },
    ApplyHousingPreset {
        path: String,
        scope: HousingPresetScope,
        reload: bool,
    },
    ApplyLatestHousingPreset {
        scope: HousingPresetScope,
        reload: bool,
    },
    RepeatHousingPreset {
        reload: bool,
    },
    CheckHousingPreset {
        path: String,
        scope: HousingPresetScope,
    },
    CheckLatestHousingPreset {
        scope: HousingPresetScope,
    },
    CheckRepeatedHousingPreset {},
    GiveHousingKit {
        kit: HousingKit,
    },
    EnterTestApartment {
        room_number: u16,
    },
    EnterTestHouse {},
    ExitTestHouse {},
    ReloadHousing {},
    UnlockContent {
        id: u16,
    },
    UnlockAllContent {},
    AddExp {
        amount: i32,
    },
    StartEvent {
        event_id: u32,
        event_type: EventType,
        event_arg: u32,
    },
    SetInnWakeup {
        watched: bool,
    },
    ToggleMount {
        id: u32,
    },
    MoveToPopRange {
        id: u32,
        fade_out: bool,
    },
    SetHP {
        hp: u32,
    },
    SetMP {
        mp: u16,
    },
    ToggleGlassesStyle {
        id: u32,
    },
    ToggleGlassesStyleAll {},
    ToggleOrnament {
        id: u32,
    },
    ToggleOrnamentAll {},
    UnlockBuddyEquip {
        id: u32,
    },
    UnlockBuddyEquipAll {},
    ToggleChocoboTaxiStand {
        id: u32,
    },
    ToggleChocoboTaxiStandAll {},
    ToggleCaughtFish {
        id: u32,
    },
    ToggleCaughtFishAll {},
    ToggleCaughtSpearfish {
        id: u32,
    },
    ToggleCaughtSpearfishAll {},
    ToggleTripleTriadCard {
        id: u32,
    },
    ToggleTripleTriadCardAll {},
    ToggleAdventure {
        id: u32,
    },
    ToggleAdventureAll {},
    ToggleCutsceneSeen {
        id: u32,
        value: bool,
    },
    ToggleCutsceneSeenAll {},
    ToggleMinion {
        id: u32,
    },
    ToggleMinionAll {},
    ToggleAetherCurrent {
        id: u32,
    },
    ToggleAetherCurrentAll {},
    ToggleAetherCurrentCompFlgSet {
        id: u32,
    },
    ToggleAetherCurrentCompFlgSetAll {},
    SetRace {
        race: u8,
    },
    SetTribe {
        tribe: u8,
    },
    SetSex {
        sex: u8,
    },
    // previously, this was kept as a separate thing apart from tasks
    // but I discovered that this doesn't mix well - for example with play_scene (segment-based) and start_event (task)
    // this is because segments were always sent before tasks and there wasn't strong ordering
    // so be careful when changing this system!
    SendSegment {
        segment: PacketSegment<ServerZoneIpcSegment>,
    },
    // NOTE: this is mostly a workaround in a limitation in the scripting API
    StartTalkEvent {},
    AcceptQuest {
        id: u32,
    },
    FinishQuest {
        id: u32,
    },
    GainStatusEffect {
        effect_id: u16,
        effect_param: u16,
        duration: f32,
    },
    RegisterForContent {
        content_id: u16,
    },
    CommenceDuty {
        director_id: u32,
    },
    QuestSequence {
        id: u32,
        sequence: u8,
    },
    CancelQuest {
        id: u32,
    },
    IncompleteQuest {
        id: u32,
    },
    Kill {},
    AbandonContent {},
    SetHomepoint {
        homepoint: u16,
    },
    ReturnToHomepoint {},
    JoinContent {
        id: u32,
    },
    FinishCastingGlamour {},
    PlayScene {
        scene: u16,
        scene_flags: SceneFlags,
        params: Vec<u32>,
    },
    ResumeEvent {
        scene: u16,
        resume_id: u8,
        params: Vec<u32>,
    },
    WarpPopRange {
        territory_id: u16,
        pop_range_id: u32,
    },
    RemoveCooldowns {},
    ToggleHowTo {
        value: bool,
        id: u32,
    },
    SendMailboxStatus {},
    SetGrandCompany {
        company: GrandCompany,
    },
    SetGrandCompanyRank {
        rank: u8,
    },
    Jump {
        name: String,
    },
    Call {
        name: String,
    },
}
