mod entry;
mod init;
mod serialization;
mod state;

use self::entry::{housing_indoor_login_exit_location, selected_or_default_local_estate};
use self::serialization::{
    FREE_COMPANY_HOUSING_FLAG, build_apartment_list_entries, build_furniture_lists,
    build_house_list_houses, build_housing_ward_info, house_exterior_from_json,
    housing_container_type_from_i32, housing_estate_greeting_from_estate,
    housing_interior_details_from_json, housing_interior_renovation_row_id_from_json,
    housing_inventory_from_rows, housing_occupied_land_info_from_estate, housing_vacant_land_info,
    owned_housing_land_data,
};
#[cfg(test)]
use self::serialization::{
    HouseExteriorJson, HouseInteriorJson, MINIMAL_INTERIOR_FLOOR, MINIMAL_INTERIOR_LIGHT,
    MINIMAL_INTERIOR_WALL, estate_land_data, housing_interior_details,
};
use glam::Vec3;
use physis::TerritoryIntendedUse;

pub use state::{
    ActiveHousingEstate, ActiveHousingWardContext, AppliedHousingAppearanceItemOperation,
    LastHousingPreset, PendingHousingAppearanceItemOperation,
};

use super::{ZoneConnection, housing_item_operation::housing_item_operation_hint};
use crate::common::{HousingFurnitureObject, HousingFurnitureObjectKey};
use crate::gamedata::GameData;
use crate::housing::apartment::{MAX_APARTMENT_ROOM_NUMBER, valid_apartment_room_number};
#[cfg(test)]
use crate::inventory::housing_container_slot_capacity;
use crate::inventory::{
    HousingInventory, Item, flat_slot_for_container, indoor_container_for_flat_slot,
    interior_placed_container_index,
};
use crate::{
    DEFAULT_LOCAL_HOUSING_DIVISION, DEFAULT_LOCAL_HOUSING_PLOT_INDEX,
    DEFAULT_LOCAL_HOUSING_WARD_INDEX, HousingEstate, HousingFurniture, HousingPlotLocation,
    ItemInfoQuery, ToServer, WorldDatabase,
    lua::{HousingExteriorColorField, HousingExteriorField, HousingInteriorField},
};
use kawari::{
    common::{
        ContainerType, HouseId, HouseUnit, ITEM_CONDITION_MAX, ItemOperationKind, Position,
        internal_housing_row,
    },
    ipc::zone::{
        ActorControlCategory, ActorSetPos, ApartmentList, ApartmentListEntry, Furniture,
        FurnitureList, HouseExterior, HouseList, HousingInteriorDetails, HousingItemOperation,
        HousingObjectDataValueSet, ItemInfo, ItemOperation, PlotSize, ServerZoneIpcData,
        ServerZoneIpcSegment,
    },
};

pub struct PersistedFurniturePlacement {
    pub container: ContainerType,
    pub slot: u16,
    pub catalog_id: u16,
    pub stain: u8,
    pub position: Position,
    pub indoors: bool,
    pub rotation: f32,
    pub plot_index: u8,
    pub object_slot: u16,
    pub spawned: bool,
}

pub struct PersistedFurnitureTranslation {
    pub storage_id: ContainerType,
    pub container_slot: u16,
    pub plot_number: u16,
    pub object_key: HousingFurnitureObjectKey,
}

pub struct PersistedHousingItemMove {
    pub item_id: u32,
    pub src_container: ContainerType,
    pub dst_container: ContainerType,
    pub ack_slot: u16,
    pub removed_from_world: bool,
    pub object_key: Option<HousingFurnitureObjectKey>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct HousingLoginExitLocation {
    zone_id: u16,
    position: Position,
    rotation: f32,
    plot_location: Option<HousingPlotLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct HousingEntryTransform {
    position: Position,
    rotation: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutdoorHousingLocation {
    territory_type_id: u16,
    ward_index: u8,
    division: u8,
    plot_index: u8,
    raw_plot_index: u8,
}

const DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_SMALL: u16 = 1249; // Simple Style cottage/small house interior
const DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_MEDIUM: u16 = 1250; // Simple Style house/medium house interior
const DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_LARGE: u16 = 1251; // Simple Style mansion/large house interior
const DEFAULT_LOCAL_APARTMENT_INDOOR_TERRITORY_TYPE_ID: u16 = 609; // Lily Hills apartment interior
const ITEM_UI_CATEGORY_ROOF: u8 = 65;
const ITEM_UI_CATEGORY_EXTERIOR_WALL: u8 = 66;
const ITEM_UI_CATEGORY_WINDOW: u8 = 67;
const ITEM_UI_CATEGORY_DOOR: u8 = 68;
const ITEM_UI_CATEGORY_ROOF_DECORATION: u8 = 69;
const ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION: u8 = 70;
const ITEM_UI_CATEGORY_PLACARD: u8 = 71;
const ITEM_UI_CATEGORY_FENCE: u8 = 72;
const ITEM_UI_CATEGORY_INTERIOR_WALL: u8 = 73;
const ITEM_UI_CATEGORY_FLOORING: u8 = 74;
const ITEM_UI_CATEGORY_CEILING_LIGHT: u8 = 75;
const INDOOR_FURNITURE_LISTS_BEFORE_FINISH_LOADING: usize = 3;
const HOUSING_PLOTS_PER_DIVISION: u8 = 30;
const APARTMENT_INTERIOR_TERRITORY_TYPE_IDS: [u16; 5] = [608, 609, 610, 655, 999];

pub(super) fn update_exterior_json_field(
    existing_json: &str,
    field: HousingExteriorField,
    value: u16,
) -> Result<String, serde_json::Error> {
    serialization::update_exterior_json_field(existing_json, field, value)
}

pub(super) fn update_exterior_json_color(
    existing_json: &str,
    field: HousingExteriorColorField,
    value: u8,
) -> Result<String, serde_json::Error> {
    serialization::update_exterior_json_color(existing_json, field, value)
}

pub(super) fn update_interior_json_field(
    existing_json: &str,
    field: HousingInteriorField,
    value: u32,
) -> Result<String, serde_json::Error> {
    serialization::update_interior_json_field(existing_json, field, value)
}

pub(super) fn update_interior_json_renovation_row_id(
    existing_json: &str,
    renovation_row_id: u16,
) -> Result<String, serde_json::Error> {
    serialization::update_interior_json_renovation_row_id(existing_json, renovation_row_id)
}

impl ZoneConnection {
    pub fn resolve_active_housing_estate(
        &mut self,
        intended_use: TerritoryIntendedUse,
        zone_id: u16,
    ) {
        self.active_housing_estate = match intended_use {
            TerritoryIntendedUse::HousingIndoor => {
                let preferred_apartment_context = Some(self.apartment_ward_context_or_default());
                let resolved = {
                    let mut database = self.database.lock();
                    resolve_active_indoor_housing_estate(
                        &mut database,
                        self.active_housing_estate.as_ref(),
                        preferred_apartment_context,
                        zone_id,
                        self.config.world_id,
                        self.player_data.character.content_id as u64,
                    )
                };
                resolved
            }
            TerritoryIntendedUse::HousingOutdoor => {
                let estate = {
                    let mut database = self.database.lock();
                    database.housing_estate_by_location(
                        zone_id,
                        self.config.world_id,
                        DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                        DEFAULT_LOCAL_HOUSING_DIVISION,
                        DEFAULT_LOCAL_HOUSING_PLOT_INDEX,
                    )
                };

                estate.map(|estate| active_housing_estate(&estate, false))
            }
            _ => None,
        };
    }

    pub fn display_housing_ward_context_or_default(&self) -> ActiveHousingWardContext {
        display_housing_ward_context_or_default(
            self.display_housing_ward_context,
            self.active_housing_ward_context,
            self.active_housing_estate_ward_context(),
            self.default_housing_ward_context(),
        )
    }

    fn active_housing_estate_ward_context(&self) -> Option<ActiveHousingWardContext> {
        let active_estate = self.active_housing_estate.as_ref()?;
        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(active_estate.house_id)
        }?;

        Some(active_housing_ward_context_from_estate(&estate))
    }

    fn default_housing_ward_context(&self) -> ActiveHousingWardContext {
        ActiveHousingWardContext {
            territory_type_id: self.player_data.volatile.zone_id as u16,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        }
    }

    pub(crate) fn apartment_ward_context_or_default(&self) -> ActiveHousingWardContext {
        let context = self.display_housing_ward_context_or_default();
        if internal_housing_row(context.territory_type_id).is_some() {
            return context;
        }

        ActiveHousingWardContext {
            territory_type_id: crate::DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        }
    }

    pub fn set_active_housing_ward_context_from_estate(&mut self, estate: &HousingEstate) {
        let context = active_housing_ward_context_from_estate(estate);
        self.active_housing_ward_context = Some(context);
        self.display_housing_ward_context = Some(context);
    }

    pub(crate) fn set_active_housing_estate_from_row(
        &mut self,
        estate: &HousingEstate,
        indoors: bool,
    ) {
        self.active_housing_estate = Some(active_housing_estate(estate, indoors));
        self.set_active_housing_ward_context_from_estate(estate);
    }

    pub(crate) fn clear_housing_furniture_reset_cache(&mut self) {
        clear_housing_reset_cache(
            &mut self.active_housing_estate,
            &mut self.player_data.house_inventory,
            false,
        );
    }

    pub(crate) fn clear_housing_estate_reset_cache(&mut self) {
        clear_housing_reset_cache(
            &mut self.active_housing_estate,
            &mut self.player_data.house_inventory,
            true,
        );
    }

    pub(crate) fn selected_or_owned_housing_estate(&mut self) -> Option<HousingEstate> {
        let active_estate = self.active_housing_estate.clone();
        let mut database = self.database.lock();
        selected_or_owned_housing_estate(
            &mut database,
            active_estate.as_ref(),
            self.player_data.character.content_id as u64,
        )
    }

    pub(crate) fn normalize_housing_indoor_login_location(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        self.pending_housing_login_exit_plot_location = None;
        let active_estate = self.active_housing_estate.clone();
        let estate = {
            let mut database = self.database.lock();
            selected_or_owned_housing_estate(
                &mut database,
                active_estate.as_ref(),
                self.player_data.character.content_id as u64,
            )
        };
        let Some(location) = housing_indoor_login_exit_location(intended_use, estate.as_ref())
        else {
            return false;
        };

        if let Some(estate) = estate.as_ref() {
            self.set_active_housing_estate_from_row(estate, false);
            tracing::debug!(
                content_id = self.player_data.character.content_id,
                land_ident = estate.land_ident,
                from_zone_id = self.player_data.volatile.zone_id,
                to_zone_id = location.zone_id,
                "Normalizing housing indoor login to outdoor front door"
            );
        }

        self.player_data.volatile.zone_id = location.zone_id as i32;
        self.player_data.volatile.position = location.position;
        self.player_data.volatile.rotation = location.rotation as f64;
        self.pending_housing_login_exit_plot_location = location.plot_location;
        true
    }

    pub async fn send_owned_housing(&mut self) {
        let estates = {
            let mut database = self.database.lock();
            database.owned_housing_estates(self.player_data.character.content_id as u64)
        };
        let (owned, apartment) = owned_housing_land_data(&estates);

        tracing::debug!(
            content_id = self.player_data.character.content_id,
            estate_count = estates.len(),
            "Sending owned housing list"
        );

        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::OwnedHousing {
            free_company_estate: owned[0],
            personal_estate: owned[1],
            personal_chambers: owned[2],
            shared_estate_1: owned[3],
            shared_estate_2: owned[4],
            apartment,
        });
        self.send_ipc_self(ipc).await;
    }

    pub async fn send_apartment_list(&mut self, starting_index: u32) {
        let context = self.apartment_ward_context_or_default();
        let mut apartments = {
            let mut database = self.database.lock();
            let rows = database.housing_apartments_by_ward(
                context.territory_type_id,
                self.config.world_id,
                context.ward_index,
                context.division,
            );
            build_apartment_list_entries(&rows, starting_index)
        };
        apartments.resize(ApartmentListEntry::COUNT, ApartmentListEntry::default());

        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::ApartmentList(ApartmentList {
            content_id: self.player_data.character.content_id as u64,
            flags: 0x80 | u16::from(context.division != 0),
            ward_id: context.ward_index as u16,
            zone_id: context.territory_type_id,
            world_id: self.config.world_id,
            list_index: starting_index,
            apartments,
        }));
        self.send_ipc_self(ipc).await;
    }

    pub async fn send_housing_ward_info(&mut self, zone_id: u16, ward_index: u8) {
        let (main_estates, subdivision_estates) = {
            let mut database = self.database.lock();
            database.housing_estates_by_ward_and_divisions(
                zone_id,
                self.config.world_id,
                ward_index,
            )
        };

        self.display_housing_ward_context = Some(ActiveHousingWardContext {
            territory_type_id: zone_id,
            ward_index,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        });

        let ward_info = build_housing_ward_info(
            zone_id,
            self.config.world_id,
            ward_index,
            &main_estates,
            &subdivision_estates,
        );

        self.send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::HousingWardInfo(ward_info),
        ))
        .await;
    }

    pub async fn send_housing_placard_info(
        &mut self,
        zone_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: u8,
    ) {
        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_location(
                zone_id,
                self.config.world_id,
                ward_index,
                division,
                plot_index,
            )
        };

        self.display_housing_ward_context = Some(ActiveHousingWardContext {
            territory_type_id: zone_id,
            ward_index,
            division,
        });

        if let Some(estate) = estate {
            if let Some(active_estate) =
                placard_authoritative_estate(&estate, self.player_data.character.content_id as u64)
            {
                self.set_active_housing_ward_context_from_estate(&estate);
                self.active_housing_estate = Some(active_estate);
            }
            self.send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingOccupiedLandInfo(housing_occupied_land_info_from_estate(
                    &estate,
                )),
            ))
            .await;
            self.send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingEstateGreeting(housing_estate_greeting_from_estate(
                    &estate,
                )),
            ))
            .await;
        } else {
            self.send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingVacantLandInfo(housing_vacant_land_info()),
            ))
            .await;
        }
    }

    pub async fn send_housing_estate_greeting(&mut self, house_id: HouseId) {
        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(house_id)
        };

        let Some(estate) = estate else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                house_id = house_id.to_u64(),
                "Unable to send housing estate greeting for unknown house id"
            );
            return;
        };

        self.set_active_housing_ward_context_from_estate(&estate);
        self.send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::HousingEstateGreeting(housing_estate_greeting_from_estate(&estate)),
        ))
        .await;
    }

    pub fn reload_active_housing_inventory(&mut self, intended_use: TerritoryIntendedUse) {
        if self.active_housing_estate.is_none() {
            self.resolve_active_housing_estate(
                intended_use,
                self.player_data.volatile.zone_id as u16,
            );
        }

        let Some(active_estate) = self.active_housing_estate.as_ref() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Unable to reload housing inventory without an active housing estate"
            );
            return;
        };

        let (rows, estate) = {
            let mut database = self.database.lock();
            (
                database.list_all_housing_furniture(active_estate.land_ident),
                database.housing_estate_by_house_id(active_estate.house_id),
            )
        };
        let mut house_inventory = housing_inventory_from_rows(&rows);
        if let Some(estate) = estate.as_ref() {
            let mut game_data = self.gamedata.lock();
            populate_housing_appearance_inventory(&mut house_inventory, &mut game_data, estate);
        }
        self.player_data.house_inventory = house_inventory;
    }

    pub fn active_housing_estate_for_edit(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) -> Option<ActiveHousingEstate> {
        if !matches!(
            intended_use,
            TerritoryIntendedUse::HousingIndoor | TerritoryIntendedUse::HousingOutdoor
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Rejecting housing edit outside a housing area"
            );
            return None;
        }

        if intended_use == TerritoryIntendedUse::HousingOutdoor {
            return self.active_housing_estate_for_outdoor_owner_gate();
        }

        if self.active_housing_estate.is_none() {
            self.resolve_active_housing_estate(
                intended_use,
                self.player_data.volatile.zone_id as u16,
            );
        }

        let Some(active_estate) = self.active_housing_estate.clone() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Rejecting housing edit without an active housing estate"
            );
            return None;
        };

        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(active_estate.house_id)
        };
        let Some(estate) = estate else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                house_id = active_estate.house_id.to_u64(),
                "Rejecting housing edit for an unknown estate"
            );
            return None;
        };

        if !can_edit_housing_estate(&estate, self.player_data.character.content_id as u64) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = estate.land_ident,
                owner_content_id = estate.owner_content_id,
                "Rejecting housing edit for a non-owner"
            );
            return None;
        }

        Some(active_estate)
    }

    pub fn can_edit_active_housing_estate(&mut self, intended_use: TerritoryIntendedUse) -> bool {
        match intended_use {
            TerritoryIntendedUse::HousingOutdoor => self
                .active_housing_estate_for_outdoor_owner_gate()
                .is_some(),
            _ => self.active_housing_estate_for_edit(intended_use).is_some(),
        }
    }

    fn active_housing_estate_for_interior_pattern(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) -> Option<ActiveHousingEstate> {
        match intended_use {
            TerritoryIntendedUse::HousingOutdoor => self
                .active_housing_estate_for_outdoor_owner_gate()
                .or_else(|| self.active_housing_estate_for_owned_outdoor_pattern()),
            TerritoryIntendedUse::HousingIndoor => {
                self.active_housing_estate_for_edit(TerritoryIntendedUse::HousingIndoor)
            }
            _ => {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    intended_use = intended_use as u8,
                    "Rejecting interior design pattern request outside a housing estate"
                );
                None
            }
        }
    }

    fn active_housing_estate_for_owned_outdoor_pattern(&mut self) -> Option<ActiveHousingEstate> {
        let active_estate = self.active_housing_estate.clone();
        let estate = {
            let mut database = self.database.lock();
            active_housing_estate_for_owned_outdoor_pattern(
                &mut database,
                active_estate.as_ref(),
                self.player_data.character.content_id as u64,
            )
            .and_then(|active_estate| database.housing_estate_by_house_id(active_estate.house_id))
        };
        let Some(estate) = estate else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                "Rejecting outdoor interior design pattern without an owned house fallback"
            );
            return None;
        };

        self.set_active_housing_estate_from_row(&estate, false);
        self.active_housing_estate.clone()
    }

    pub fn can_use_active_housing_interior_pattern(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        housing_interior_pattern_area_allows_request(intended_use)
            && self
                .active_housing_estate_for_interior_pattern(intended_use)
                .is_some()
    }

    pub fn should_reload_after_housing_interior_pattern_apply(
        &self,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        housing_interior_pattern_apply_should_reload(intended_use)
    }

    fn housing_indoor_territory_type_id_for_estate(&mut self, estate: &HousingEstate) -> u16 {
        if let Some(renovation_row_id) =
            housing_interior_renovation_row_id_from_json(&estate.interior_json)
        {
            let mut gamedata = self.gamedata.lock();
            if let Some(territory_type_id) =
                gamedata.get_housing_renovation_territory(renovation_row_id)
            {
                return territory_type_id;
            }
        }

        housing_default_indoor_entry_territory_type_id_for_estate(estate)
    }

    pub fn current_housing_interior_pattern_context(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) -> Option<(u16, u8)> {
        let active_estate = self.active_housing_estate_for_interior_pattern(intended_use)?;
        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(active_estate.house_id)
        }?;

        if estate.is_apartment {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                "Rejecting interior design pattern request for an apartment"
            );
            return None;
        }

        let plot_size = housing_estate_plot_size(&estate);
        if let Some(renovation_row_id) =
            housing_interior_renovation_row_id_from_json(&estate.interior_json)
        {
            let mut gamedata = self.gamedata.lock();
            if let Some(territory_type_id) =
                gamedata.get_housing_renovation_territory(renovation_row_id)
            {
                if let Some(current_size) =
                    gamedata.get_housing_indoor_territory_category(territory_type_id)
                {
                    return Some((renovation_row_id, current_size));
                }
            }
        }

        let current_zone_id = self.player_data.volatile.zone_id as u16;
        let fallback_zone_id = simple_housing_indoor_territory_type_id(plot_size);
        let mut gamedata = self.gamedata.lock();

        for territory_type_id in [current_zone_id, fallback_zone_id] {
            if territory_type_id == 0 {
                continue;
            }

            let Some(renovation_row_id) =
                gamedata.get_housing_renovation_row_id_for_territory(territory_type_id)
            else {
                continue;
            };

            let current_size = gamedata
                .get_housing_indoor_territory_category(territory_type_id)
                .unwrap_or(plot_size as u8);

            return Some((renovation_row_id, current_size));
        }

        tracing::warn!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            current_zone_id,
            fallback_zone_id,
            "Unable to resolve HousingRenovation row for active interior"
        );
        None
    }

    pub fn apply_housing_interior_pattern(
        &mut self,
        intended_use: TerritoryIntendedUse,
        renovation_row_id: u32,
    ) -> Option<u16> {
        let renovation_row_id = u16::try_from(renovation_row_id).ok()?;
        let active_estate = self.active_housing_estate_for_interior_pattern(intended_use)?;
        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(active_estate.house_id)
        }?;

        if estate.is_apartment {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                renovation_row_id,
                "Rejecting interior design pattern apply for an apartment"
            );
            return None;
        }

        let (territory_type_id, selected_size) = {
            let mut gamedata = self.gamedata.lock();
            let Some(territory_type_id) =
                gamedata.get_housing_renovation_territory(renovation_row_id)
            else {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    land_ident = active_estate.land_ident,
                    renovation_row_id,
                    "Rejecting unknown HousingRenovation row"
                );
                return None;
            };
            let Some(selected_size) =
                gamedata.get_housing_indoor_territory_category(territory_type_id)
            else {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    land_ident = active_estate.land_ident,
                    renovation_row_id,
                    territory_type_id,
                    "Rejecting HousingRenovation row without HousingIndoorTerritory category"
                );
                return None;
            };
            (territory_type_id, selected_size)
        };

        let current_size = self
            .current_housing_interior_pattern_context(intended_use)
            .map(|(_, size)| size)
            .unwrap_or_else(|| housing_estate_plot_size(&estate) as u8);
        if selected_size != current_size {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                renovation_row_id,
                territory_type_id,
                selected_size,
                current_size,
                "Rejecting interior design pattern with a mismatched house size"
            );
            return None;
        }

        let Ok(interior_json) =
            update_interior_json_renovation_row_id(&estate.interior_json, renovation_row_id)
        else {
            return None;
        };
        let updated = {
            let mut database = self.database.lock();
            database.update_housing_interior_json(active_estate.land_ident, &interior_json)
        };
        if !updated {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                renovation_row_id,
                territory_type_id,
                "Failed to persist interior design pattern"
            );
            return None;
        }

        tracing::info!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            renovation_row_id,
            territory_type_id,
            "Persisted interior design pattern"
        );

        Some(territory_type_id)
    }

    pub async fn reload_housing_interior_pattern_territory(&mut self, territory_type_id: u16) {
        let entry = housing_indoor_entry_transform(false);
        self.change_zone(
            territory_type_id,
            Some(entry.position),
            Some(entry.rotation),
            None,
        )
        .await;
    }

    pub fn active_housing_estate_for_outdoor_owner_gate(&mut self) -> Option<ActiveHousingEstate> {
        let Some(active_estate) = self.active_housing_estate.clone() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                "Rejecting outdoor housing owner gate without an active outdoor estate"
            );
            return None;
        };

        let resolved = {
            let mut database = self.database.lock();
            resolve_active_housing_estate_for_outdoor_owner_gate(
                &mut database,
                self.active_housing_ward_context,
                self.config.world_id,
                &active_estate,
                self.player_data.character.content_id as u64,
            )
        };
        let Some((active_estate, location)) = resolved else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                house_id = active_estate.house_id.to_u64(),
                "Rejecting outdoor housing owner gate for a missing, invalid, non-owner, or mismatched active estate"
            );
            return None;
        };

        self.active_housing_ward_context = Some(ActiveHousingWardContext {
            territory_type_id: location.territory_type_id,
            ward_index: location.ward_index,
            division: location.division,
        });
        self.active_housing_estate = Some(active_estate.clone());

        Some(active_estate)
    }

    pub fn active_housing_estate_for_outdoor_edit(
        &mut self,
        raw_plot_index: u8,
    ) -> Option<ActiveHousingEstate> {
        let resolved = {
            let mut database = self.database.lock();
            resolve_active_housing_estate_for_outdoor_edit(
                &mut database,
                self.active_housing_ward_context,
                self.config.world_id,
                raw_plot_index,
                self.player_data.character.content_id as u64,
            )
        };
        let Some((active_estate, location)) = resolved else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                raw_plot_index,
                "Rejecting outdoor housing edit without a real active ward context or matching owner estate"
            );
            return None;
        };

        self.active_housing_ward_context = Some(ActiveHousingWardContext {
            territory_type_id: location.territory_type_id,
            ward_index: location.ward_index,
            division: location.division,
        });
        self.active_housing_estate = Some(active_estate.clone());

        Some(active_estate)
    }

    pub fn active_housing_estate_for_outdoor_item_removal(
        &mut self,
    ) -> Option<ActiveHousingEstate> {
        let Some(active_estate) = self.active_housing_estate.clone() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                "Rejecting outdoor housing item removal without an active outdoor estate"
            );
            return None;
        };

        let Some(context) = self.active_housing_ward_context else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                house_id = active_estate.house_id.to_u64(),
                "Rejecting outdoor housing item removal without an active ward context"
            );
            return None;
        };

        let Some(location) = outdoor_housing_location_from_raw_entry(
            context,
            active_estate.house_id.unit.apartment_division_plot_index,
        ) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                house_id = active_estate.house_id.to_u64(),
                "Rejecting outdoor housing item removal for an invalid active house id"
            );
            return None;
        };

        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_location(
                location.territory_type_id,
                self.config.world_id,
                location.ward_index,
                location.division,
                location.plot_index,
            )
        };

        let Some(active_estate) = active_housing_estate_for_outdoor_removal_row(
            Some(&active_estate),
            estate.as_ref(),
            self.player_data.character.content_id as u64,
        ) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                house_id = active_estate.house_id.to_u64(),
                "Rejecting outdoor housing item removal for a missing, indoor, non-owner, or mismatched estate"
            );
            return None;
        };

        self.active_housing_ward_context = Some(ActiveHousingWardContext {
            territory_type_id: location.territory_type_id,
            ward_index: location.ward_index,
            division: location.division,
        });
        self.active_housing_estate = Some(active_estate.clone());

        Some(active_estate)
    }

    pub fn persist_housing_light_level(
        &mut self,
        level: u32,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        let Some(active_estate) = self.active_housing_estate_for_edit(intended_use) else {
            return false;
        };

        let updated = {
            let mut database = self.database.lock();
            database.update_housing_light_level(
                active_estate.land_ident,
                level.min(u8::MAX as u32) as u8,
            )
        };
        if !updated {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                level,
                "Housing light level update did not match a persisted estate"
            );
        }

        updated
    }

    pub async fn process_housing_appearance_item_operation(
        &mut self,
        action: &ItemOperation,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        if !housing_appearance_operation_involves_staging(action) {
            return false;
        }

        if !self.in_housing_area(intended_use)
            || !self.can_edit_active_housing_estate(intended_use)
            || !housing_appearance_operation_matches_area(action, intended_use)
        {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                src_storage = ?action.src_storage_id,
                dst_storage = ?action.dst_storage_id,
                "Rejecting housing appearance item operation outside the matching editable estate"
            );
            return true;
        }

        if !matches!(
            action.operation_type,
            ItemOperationKind::Move | ItemOperationKind::Exchange
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                operation_type = ?action.operation_type,
                "Ignoring unsupported housing appearance item operation"
            );
            return true;
        }

        let Some(src_item) =
            self.get_player_or_housing_item(action.src_storage_id, action.src_container_index)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                src_storage = ?action.src_storage_id,
                src_slot = action.src_container_index,
                "Housing appearance item operation referenced an invalid source slot"
            );
            return true;
        };

        let Some(dst_item) =
            self.get_player_or_housing_item(action.dst_storage_id, action.dst_container_index)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                dst_storage = ?action.dst_storage_id,
                dst_slot = action.dst_container_index,
                "Housing appearance item operation referenced an invalid destination slot"
            );
            return true;
        };

        if !self.set_player_or_housing_item(
            action.dst_storage_id,
            action.dst_container_index,
            src_item,
        ) || !self.set_player_or_housing_item(
            action.src_storage_id,
            action.src_container_index,
            dst_item,
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                src_storage = ?action.src_storage_id,
                dst_storage = ?action.dst_storage_id,
                "Housing appearance item operation failed while mutating local inventory state"
            );
            return true;
        }

        {
            let mut database = self.database.lock();
            database.commit_classjob_and_inventory(&self.player_data);
        }

        self.send_affected_containers(action.src_storage_id, action.dst_storage_id)
            .await;

        tracing::info!(
            content_id = self.player_data.character.content_id,
            src_storage = ?action.src_storage_id,
            src_slot = action.src_container_index,
            dst_storage = ?action.dst_storage_id,
            dst_slot = action.dst_container_index,
            "Processed housing appearance item operation"
        );

        true
    }

    pub fn record_housing_appearance_item_operation_marker(
        &mut self,
        action: &HousingItemOperation,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        let Some(operation_hint) = housing_item_operation_hint(action) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                raw = ?action.raw,
                "Ignoring housing appearance item operation without a usable source container hint"
            );
            return true;
        };
        let source_container = operation_hint.source_container;
        let source_slot = operation_hint.source_slot;

        if !self.in_housing_area(intended_use) || !self.can_edit_active_housing_estate(intended_use)
        {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Rejecting housing appearance item operation marker outside an editable estate"
            );
            return true;
        }

        let Some(source_item) = self
            .player_data
            .inventory
            .get_item(source_container, source_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?source_container,
                source_slot,
                "Housing appearance item operation marker referenced an invalid player inventory slot"
            );
            return true;
        };
        if source_item.is_empty_slot() {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?source_container,
                source_slot,
                "Housing appearance item operation marker referenced an empty player inventory slot"
            );
            return true;
        }

        let Some((_, item_ui_category)) = ({
            let mut gamedata = self.gamedata.lock();
            housing_appearance_item_data(&mut gamedata, source_item)
        }) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = source_item.item_id,
                "Housing appearance item operation marker source item is not a usable appearance fixture"
            );
            return true;
        };

        let marker_target_slot = Some(operation_hint.target_appearance_slot);
        let Some((target_container, target_slot)) =
            self.housing_appearance_target(item_ui_category, intended_use, marker_target_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = source_item.item_id,
                item_ui_category,
                intended_use = intended_use as u8,
                ?marker_target_slot,
                "Housing appearance item operation marker has no supported target slot"
            );
            return true;
        };

        self.pending_housing_appearance_item_operation =
            Some(PendingHousingAppearanceItemOperation {
                source_container,
                source_slot,
                target_container,
                target_slot,
            });

        tracing::info!(
            content_id = self.player_data.character.content_id,
            ?source_container,
            source_slot,
            item_id = source_item.item_id,
            item_ui_category,
            ?marker_target_slot,
            ?target_container,
            target_slot,
            "Staged housing appearance item operation marker"
        );

        true
    }

    pub fn clear_pending_housing_appearance_item_operation(&mut self) {
        self.pending_housing_appearance_item_operation = None;
    }

    pub fn apply_pending_housing_appearance_item_operation(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) -> Option<AppliedHousingAppearanceItemOperation> {
        let Some(operation) = self.pending_housing_appearance_item_operation.take() else {
            return None;
        };

        if housing_appearance_container_intended_use(operation.target_container)
            != Some(intended_use)
            || !self.in_housing_area(intended_use)
            || !self.can_edit_active_housing_estate(intended_use)
        {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                ?operation,
                "Rejecting staged housing appearance item operation outside its editable estate"
            );
            return None;
        }

        let Some(original_source_item) = self
            .player_data
            .inventory
            .get_item(operation.source_container, operation.source_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Staged housing appearance item operation lost its source item"
            );
            return None;
        };
        let Some(original_target_item) = self
            .player_data
            .house_inventory
            .get_item(operation.target_container, operation.target_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Staged housing appearance item operation lost its target slot"
            );
            return None;
        };

        if original_source_item.is_empty_slot() {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Staged housing appearance item operation source item is empty at apply time"
            );
            return None;
        }

        if !self.housing_appearance_item_matches_target_slot(
            original_source_item,
            operation.target_container,
            operation.target_slot,
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = original_source_item.item_id,
                ?operation,
                "Staged housing appearance item operation source item does not match the target slot category"
            );
            return None;
        }

        if !self.set_player_or_housing_item(
            operation.source_container,
            operation.source_slot,
            original_target_item,
        ) || !self.set_player_or_housing_item(
            operation.target_container,
            operation.target_slot,
            original_source_item,
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Failed to apply staged housing appearance item operation"
            );
            return None;
        }

        tracing::info!(
            content_id = self.player_data.character.content_id,
            ?operation,
            source_item_id = original_source_item.item_id,
            target_item_id = original_target_item.item_id,
            "Applied staged housing appearance item operation"
        );

        Some(AppliedHousingAppearanceItemOperation {
            source_container: operation.source_container,
            source_slot: operation.source_slot,
            target_container: operation.target_container,
            target_slot: operation.target_slot,
            original_source_item,
            original_target_item,
        })
    }

    pub fn rollback_housing_appearance_item_operation(
        &mut self,
        operation: AppliedHousingAppearanceItemOperation,
    ) {
        let _ = self.set_player_or_housing_item(
            operation.source_container,
            operation.source_slot,
            operation.original_source_item,
        );
        let _ = self.set_player_or_housing_item(
            operation.target_container,
            operation.target_slot,
            operation.original_target_item,
        );
    }

    pub async fn send_housing_appearance_item_operation_update(
        &mut self,
        operation: AppliedHousingAppearanceItemOperation,
        intended_use: TerritoryIntendedUse,
    ) {
        let source_update =
            ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
                sequence: self.player_data.item_sequence,
                container: operation.source_container,
                slot: operation.source_slot,
                ..operation.original_target_item.into()
            }));
        self.send_ipc_self(source_update).await;
        self.player_data.item_sequence += 1;

        let target_update =
            ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
                sequence: self.player_data.item_sequence,
                container: operation.target_container,
                slot: operation.target_slot,
                ..operation.original_source_item.into()
            }));
        self.send_ipc_self(target_update).await;
        self.player_data.item_sequence += 1;

        self.actor_control_self(ActorControlCategory::FinishEstateAppearanceItemOperation {})
            .await;

        if intended_use == TerritoryIntendedUse::HousingIndoor {
            self.send_current_housing_interior_details().await;
        }
    }

    pub fn persist_housing_appearance_remodel(
        &mut self,
        apply: bool,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        if !apply {
            self.reload_active_housing_inventory(intended_use);
            tracing::info!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Canceled housing appearance remodel"
            );
            return true;
        }

        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_owner_gate()
        } else {
            self.active_housing_estate_for_edit(intended_use)
        };
        let Some(active_estate) = active_estate else {
            return false;
        };

        let Some(estate) = ({
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(active_estate.house_id)
        }) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                house_id = active_estate.house_id.to_u64(),
                "Rejecting housing appearance remodel for an unknown estate"
            );
            return false;
        };

        let updated = match intended_use {
            TerritoryIntendedUse::HousingIndoor => {
                let Some(interior_json) = self.interior_json_from_current_appearance(&estate)
                else {
                    return false;
                };
                let mut database = self.database.lock();
                database.update_housing_interior_json(active_estate.land_ident, &interior_json)
            }
            TerritoryIntendedUse::HousingOutdoor => {
                let Some(exterior_json) = self.exterior_json_from_current_appearance(&estate)
                else {
                    return false;
                };
                let mut database = self.database.lock();
                database.update_housing_exterior_json(active_estate.land_ident, &exterior_json)
            }
            _ => false,
        };

        if updated {
            self.reload_active_housing_inventory(intended_use);
            tracing::info!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                intended_use = intended_use as u8,
                "Persisted housing appearance remodel"
            );
        } else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                intended_use = intended_use as u8,
                "Housing appearance remodel did not match a persisted estate"
            );
        }

        updated
    }

    fn interior_json_from_current_appearance(&mut self, estate: &HousingEstate) -> Option<String> {
        let items = self.housing_appearance_items(ContainerType::HousingInteriorAppearance);
        let mut json = estate.interior_json.clone();
        let mut gamedata = self.gamedata.lock();

        for (slot, item) in items {
            let Some(field) = housing_interior_field_for_appearance_slot(slot) else {
                continue;
            };
            let Some((additional_data, item_ui_category)) =
                housing_appearance_item_data(&mut gamedata, item)
            else {
                continue;
            };
            let expected_category = housing_interior_item_ui_category_for_slot(slot);
            if Some(item_ui_category) != expected_category {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    slot,
                    item_id = item.item_id,
                    item_ui_category,
                    ?expected_category,
                    "Skipping interior appearance item with an unexpected UI category"
                );
                continue;
            }

            json = match update_interior_json_field(&json, field, additional_data) {
                Ok(json) => json,
                Err(_) => return None,
            };
        }

        Some(json)
    }

    fn exterior_json_from_current_appearance(&mut self, estate: &HousingEstate) -> Option<String> {
        let items = self.housing_appearance_items(ContainerType::HousingExteriorAppearance);
        let mut json = estate.exterior_json.clone();
        let mut gamedata = self.gamedata.lock();

        for (slot, item) in items {
            let Some(field) = housing_exterior_field_for_appearance_slot(slot) else {
                continue;
            };
            let Some((additional_data, item_ui_category)) =
                housing_appearance_item_data(&mut gamedata, item)
            else {
                continue;
            };
            let expected_category = housing_exterior_item_ui_category_for_slot(slot);
            if Some(item_ui_category) != expected_category {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    slot,
                    item_id = item.item_id,
                    item_ui_category,
                    ?expected_category,
                    "Skipping exterior appearance item with an unexpected UI category"
                );
                continue;
            }

            json = match update_exterior_json_field(&json, field, additional_data as u16) {
                Ok(json) => json,
                Err(_) => return None,
            };
            if item.stains[0] != 0 {
                let Some(color_field) = housing_exterior_color_field_for_appearance_slot(slot)
                else {
                    continue;
                };
                json = match update_exterior_json_color(&json, color_field, item.stains[0]) {
                    Ok(json) => json,
                    Err(_) => return None,
                };
            }
        }

        Some(json)
    }

    fn housing_appearance_items(&self, container: ContainerType) -> Vec<(u16, Item)> {
        let Some(container) = self.player_data.house_inventory.get_container(container) else {
            return Vec::new();
        };

        (0..container.max_slots())
            .filter_map(|slot| {
                let item = *container.get_slot(slot as u16);
                (!item.is_empty_slot()).then_some((slot as u16, item))
            })
            .collect()
    }

    fn housing_appearance_target(
        &mut self,
        item_ui_category: u8,
        intended_use: TerritoryIntendedUse,
        marker_target_slot: Option<u16>,
    ) -> Option<(ContainerType, u16)> {
        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_owner_gate()
        } else {
            self.active_housing_estate_for_edit(intended_use)
        }?;

        let is_apartment = if intended_use == TerritoryIntendedUse::HousingIndoor {
            let mut database = self.database.lock();
            database
                .housing_estate_by_house_id(active_estate.house_id)
                .is_some_and(|estate| estate.is_apartment)
        } else {
            false
        };

        let target_container = match intended_use {
            TerritoryIntendedUse::HousingIndoor => ContainerType::HousingInteriorAppearance,
            TerritoryIntendedUse::HousingOutdoor => ContainerType::HousingExteriorAppearance,
            _ => return None,
        };
        let target_slot = housing_appearance_marker_target_slot(
            item_ui_category,
            intended_use,
            is_apartment,
            marker_target_slot,
        )
        .unwrap_or(default_housing_appearance_target_slot(
            item_ui_category,
            intended_use,
            is_apartment,
        )?);

        Some((target_container, target_slot))
    }

    fn housing_appearance_item_matches_target_slot(
        &mut self,
        item: Item,
        target_container: ContainerType,
        target_slot: u16,
    ) -> bool {
        let expected_category = match target_container {
            ContainerType::HousingInteriorAppearance
            | ContainerType::HousingInteriorAppearanceEdit => {
                housing_interior_item_ui_category_for_slot(target_slot)
            }
            ContainerType::HousingExteriorAppearance
            | ContainerType::HousingExteriorAppearanceEdit => {
                housing_exterior_item_ui_category_for_slot(target_slot)
            }
            _ => None,
        };

        let Some(expected_category) = expected_category else {
            return false;
        };
        let Some((_, item_ui_category)) = ({
            let mut gamedata = self.gamedata.lock();
            housing_appearance_item_data(&mut gamedata, item)
        }) else {
            return false;
        };

        item_ui_category == expected_category
    }

    fn get_player_or_housing_item(&self, container: ContainerType, slot: u16) -> Option<Item> {
        self.player_data
            .inventory
            .get_item(container, slot)
            .or_else(|| self.player_data.house_inventory.get_item(container, slot))
    }

    fn set_player_or_housing_item(
        &mut self,
        container: ContainerType,
        slot: u16,
        item: Item,
    ) -> bool {
        if let Some(dst) = self.player_data.inventory.get_item_mut(container, slot) {
            *dst = item;
            return true;
        }
        if let Some(dst) = self
            .player_data
            .house_inventory
            .get_item_mut(container, slot)
        {
            *dst = item;
            return true;
        }

        false
    }

    async fn send_current_housing_interior_details(&mut self) {
        let Some(active_estate) = self.active_housing_estate.as_ref() else {
            return;
        };
        let Some((interior_json, light_level, is_apartment)) = ({
            let mut database = self.database.lock();
            database
                .housing_estate_by_house_id(active_estate.house_id)
                .map(|estate| {
                    (
                        estate.interior_json,
                        estate.light_level.clamp(0, u8::MAX as i32) as u8,
                        estate.is_apartment,
                    )
                })
        }) else {
            return;
        };

        self.send_housing_interior_details(&interior_json, light_level, is_apartment)
            .await;
    }

    async fn send_housing_interior_details(
        &mut self,
        interior_json: &str,
        light_level: u8,
        is_apartment: bool,
    ) {
        let details = housing_interior_details_from_json(interior_json, light_level, is_apartment);

        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::HousingInteriorDetails(details));
        self.send_ipc_self(ipc).await;
    }

    async fn send_housing_furniture_lists(
        &mut self,
        house_id: HouseId,
        rows: &[HousingFurniture],
        indoors: bool,
        slot_capacity: Option<usize>,
    ) {
        let lists = build_furniture_lists(house_id, rows, indoors, slot_capacity);
        self.send_built_housing_furniture_lists(
            lists,
            house_id,
            rows.len(),
            indoors,
            slot_capacity,
            0,
            usize::MAX,
            "all",
        )
        .await;
    }

    async fn send_deferred_housing_furniture_lists(
        &mut self,
        house_id: HouseId,
        rows: &[HousingFurniture],
        indoors: bool,
        slot_capacity: Option<usize>,
    ) {
        let lists = build_furniture_lists(house_id, rows, indoors, slot_capacity);
        let start_index = deferred_housing_furniture_list_start_index(indoors, lists.len());
        self.send_built_housing_furniture_lists(
            lists,
            house_id,
            rows.len(),
            indoors,
            slot_capacity,
            start_index,
            usize::MAX,
            "after_remodel_gate",
        )
        .await;
    }

    async fn send_built_housing_furniture_lists(
        &mut self,
        lists: Vec<FurnitureList>,
        house_id: HouseId,
        furniture_count: usize,
        indoors: bool,
        slot_capacity: Option<usize>,
        start_index: usize,
        end_index: usize,
        phase: &'static str,
    ) {
        let list_count = lists.len();
        let start_index = start_index.min(list_count);
        let end_index = end_index.min(list_count);
        let send_count = end_index.saturating_sub(start_index);

        tracing::debug!(
            house_id = house_id.to_u64(),
            furniture_count,
            list_count,
            start_index,
            end_index,
            send_count,
            indoors,
            slot_capacity,
            phase,
            "Sending housing furniture lists"
        );

        for list in lists.into_iter().skip(start_index).take(send_count) {
            self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                list,
            )))
            .await;
        }
    }

    pub async fn persist_furniture_placement(
        &mut self,
        container: ContainerType,
        slot: u16,
        position: Position,
        rotation: f32,
        spawn_furniture: bool,
        plot_index: u8,
        intended_use: TerritoryIntendedUse,
    ) -> Option<PersistedFurniturePlacement> {
        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_edit(plot_index)
        } else {
            self.active_housing_estate_for_edit(intended_use)
        };
        let Some(active_estate) = active_estate else {
            return None;
        };

        let transfer_item = match self.housing_transfer_item(container, slot) {
            Some(item) if !item.is_empty_slot() => item,
            _ => {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    ?container,
                    slot,
                    "Rejecting furniture placement from an empty or invalid source slot"
                );
                return None;
            }
        };

        let catalog_id = {
            let gamedata = self.gamedata.lock();
            gamedata
                .get_furniture_catalog_id(transfer_item.item_id)
                .unwrap_or_default()
        };
        if catalog_id == 0 {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = transfer_item.item_id,
                "Furniture catalog id resolved to zero"
            );
        }

        let desired_pages = self
            .player_data
            .house_inventory
            .get_desired_pages_from_intendeduse(intended_use, !spawn_furniture);
        let Some(result) = self
            .player_data
            .house_inventory
            .add_in_empty_slot(transfer_item, desired_pages)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?desired_pages,
                "Rejecting furniture placement because the target housing inventory is full"
            );
            return None;
        };
        let object_slot = if spawn_furniture {
            let Some(object_slot) = flat_slot_for_container(result.container, result.slot) else {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    dst_container = ?result.container,
                    dst_slot = result.slot,
                    "Rejecting spawned furniture placement because destination is not a placed furniture container"
                );
                return None;
            };
            object_slot
        } else {
            0
        };

        self.clear_housing_transfer_source(container, slot).await?;

        let src_container_type = container;
        let dst_container_type = result.container;
        let stain = transfer_item.stains[0];
        {
            let mut database = self.database.lock();
            database.upsert_housing_furniture(HousingFurniture {
                land_ident: active_estate.land_ident,
                container_type: result.container as u16 as i32,
                slot: result.slot as i32,
                item_id: transfer_item.item_id as i64,
                catalog_id: catalog_id as i32,
                stain: stain as i32,
                placed: spawn_furniture,
                pos_x: position.0.x,
                pos_y: position.0.y,
                pos_z: position.0.z,
                rotation,
                created_by_content_id: Some(self.player_data.character.content_id),
                ..Default::default()
            });
            database.commit_classjob_and_inventory(&self.player_data);
        }

        self.send_affected_containers(src_container_type, dst_container_type)
            .await;

        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            ?container,
            slot,
            dst_container = ?result.container,
            dst_slot = result.slot,
            catalog_id,
            spawn_furniture,
            "Persisted housing furniture placement"
        );

        Some(PersistedFurniturePlacement {
            container: result.container,
            slot: result.slot,
            catalog_id,
            stain,
            position,
            indoors: intended_use == TerritoryIntendedUse::HousingIndoor,
            rotation,
            plot_index,
            object_slot,
            spawned: spawn_furniture,
        })
    }

    pub fn persist_furniture_translation(
        &mut self,
        house_id: HouseId,
        flat_slot: u16,
        position: Position,
        rotation: f32,
        intended_use: TerritoryIntendedUse,
    ) -> Option<PersistedFurnitureTranslation> {
        let indoors = intended_use == TerritoryIntendedUse::HousingIndoor;
        let Some((storage_id, container_slot)) = placed_container_for_flat_slot(flat_slot, indoors)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                flat_slot,
                indoors,
                "Rejecting furniture translation for an out-of-range slot"
            );
            return None;
        };

        let active_estate = if indoors {
            self.active_housing_estate_for_edit(intended_use)
        } else {
            self.active_housing_estate_for_outdoor_edit(house_id.unit.apartment_division_plot_index)
        };
        let Some(active_estate) = active_estate else {
            return None;
        };

        if active_estate.house_id != house_id {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                active_house_id = active_estate.house_id.to_u64(),
                packet_house_id = house_id.to_u64(),
                "Rejecting furniture translation for a different house id than the active estate"
            );
            return None;
        }

        let updated = {
            let mut database = self.database.lock();
            database.update_housing_furniture_position(
                active_estate.land_ident,
                storage_id,
                container_slot,
                position,
                rotation,
            )
        };
        if !updated {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                ?storage_id,
                container_slot,
                "Furniture translation did not match a persisted furniture row"
            );
        }

        Some(PersistedFurnitureTranslation {
            storage_id,
            container_slot,
            plot_number: if !house_id.unit.apartment_flag {
                house_id.unit.apartment_division_plot_index as u16
            } else {
                0
            },
            object_key: HousingFurnitureObjectKey {
                slot: flat_slot,
                indoors,
                plot_index: house_id.unit.apartment_division_plot_index,
            },
        })
    }

    pub async fn persist_housing_item_move_to_inventory(
        &mut self,
        to_storeroom: bool,
        storage_id: ContainerType,
        slot: u16,
        intended_use: TerritoryIntendedUse,
    ) -> Option<PersistedHousingItemMove> {
        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_item_removal()
        } else {
            self.active_housing_estate_for_edit(intended_use)
        };
        let Some(active_estate) = active_estate else {
            return None;
        };

        let Some(transfer_item) = self.player_data.house_inventory.get_item(storage_id, slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?storage_id,
                slot,
                "Rejecting housing item move from an invalid source slot"
            );
            return None;
        };
        if transfer_item.is_empty_slot() {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?storage_id,
                slot,
                "Rejecting housing item move from an empty source slot"
            );
            return None;
        }
        let item_id = transfer_item.item_id;
        let removed_from_world = should_broadcast_housing_item_removal(storage_id);
        let object_key = if removed_from_world {
            flat_slot_for_container(storage_id, slot).map(|flat_slot| HousingFurnitureObjectKey {
                slot: flat_slot,
                indoors: intended_use == TerritoryIntendedUse::HousingIndoor,
                plot_index: active_estate.house_id.unit.apartment_division_plot_index,
            })
        } else {
            None
        };

        let desired_pages = self
            .player_data
            .house_inventory
            .get_desired_pages_from_intendeduse(intended_use, to_storeroom);
        let item_info = if !to_storeroom {
            self.player_data
                .inventory
                .add_in_next_free_slot(transfer_item)
        } else {
            self.player_data
                .house_inventory
                .add_in_empty_slot(transfer_item, desired_pages)
        };
        let Some(item_info) = item_info else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?desired_pages,
                to_storeroom,
                "Rejecting housing item move because destination inventory is full"
            );
            return None;
        };

        let source_slot = self
            .player_data
            .house_inventory
            .get_item_mut(storage_id, slot)?;
        *source_slot = Item::default();

        {
            let mut database = self.database.lock();
            let moved = if to_storeroom {
                database.move_housing_furniture_to_container(
                    active_estate.land_ident,
                    storage_id,
                    slot,
                    Some(item_info.container),
                    Some(item_info.slot),
                    false,
                )
            } else {
                database.move_housing_furniture_to_container(
                    active_estate.land_ident,
                    storage_id,
                    slot,
                    None,
                    None,
                    false,
                )
            };

            if !moved {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    land_ident = active_estate.land_ident,
                    ?storage_id,
                    slot,
                    to_storeroom,
                    "Housing item move did not match a persisted furniture row"
                );
            }

            database.commit_classjob_and_inventory(&self.player_data);
        }

        self.send_affected_containers(storage_id, item_info.container)
            .await;

        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            ?storage_id,
            slot,
            dst_container = ?item_info.container,
            dst_slot = item_info.slot,
            to_storeroom,
            "Persisted housing item move"
        );

        Some(PersistedHousingItemMove {
            item_id,
            src_container: storage_id,
            dst_container: item_info.container,
            ack_slot: slot,
            removed_from_world,
            object_key,
        })
    }

    pub async fn move_to_housing_front_door(&mut self) {
        if self.active_housing_estate.is_none() {
            self.resolve_active_housing_estate(
                TerritoryIntendedUse::HousingIndoor,
                self.player_data.volatile.zone_id as u16,
            );
        }

        let Some(active_estate) = self.active_housing_estate.clone() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                "Rejecting housing front-door move without an active housing estate"
            );
            return;
        };

        let entry = housing_indoor_entry_transform(false);
        self.player_data.volatile.position = entry.position;
        self.player_data.volatile.rotation = entry.rotation as f64;

        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            "Moving player to housing front door"
        );

        self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::ActorSetPos(
            ActorSetPos {
                position: entry.position,
                rotation: entry.rotation,
                ..Default::default()
            },
        )))
        .await;
    }

    pub async fn reload_current_housing_interior(&mut self) {
        let active_estate = self.active_housing_estate.clone();
        let estate = {
            let mut database = self.database.lock();
            selected_or_default_local_estate(
                &mut database,
                active_estate.as_ref(),
                self.player_data.character.content_id as u64,
                &self.player_data.character.name,
                self.config.world_id,
            )
        };

        self.set_active_housing_estate_from_row(&estate, true);

        let is_apartment = estate.is_apartment && estate.room_number > 0;
        let entry = housing_indoor_entry_transform(is_apartment);
        let indoor_territory_type_id = if is_apartment {
            DEFAULT_LOCAL_APARTMENT_INDOOR_TERRITORY_TYPE_ID
        } else {
            self.housing_indoor_territory_type_id_for_estate(&estate)
        };
        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = estate.land_ident,
            house_id = estate.house_id,
            territory_type_id = indoor_territory_type_id,
            is_apartment,
            "Reloading current housing interior"
        );

        self.change_zone(
            indoor_territory_type_id,
            Some(entry.position),
            Some(entry.rotation),
            None,
        )
        .await;
    }

    fn housing_transfer_item(&self, container: ContainerType, slot: u16) -> Option<Item> {
        self.player_data
            .inventory
            .get_item(container, slot)
            .or_else(|| self.player_data.house_inventory.get_item(container, slot))
    }

    async fn clear_housing_transfer_source(
        &mut self,
        container: ContainerType,
        slot: u16,
    ) -> Option<()> {
        let old_slot = if let Some(item) = self.player_data.inventory.get_item_mut(container, slot)
        {
            item
        } else if let Some(item) = self
            .player_data
            .house_inventory
            .get_item_mut(container, slot)
        {
            item
        } else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?container,
                slot,
                "Unable to clear housing furniture source slot"
            );
            return None;
        };

        *old_slot = Item::default();

        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
            sequence: 0,
            container,
            slot,
            ..Item::default().into()
        }));
        self.send_ipc_self(ipc).await;

        Some(())
    }
}

fn active_housing_estate(estate: &HousingEstate, indoors: bool) -> ActiveHousingEstate {
    ActiveHousingEstate {
        land_ident: estate.land_ident,
        house_id: HouseId::from_u64(estate.house_id as u64),
        indoors,
    }
}

fn selected_or_owned_housing_estate(
    database: &mut WorldDatabase,
    active_estate: Option<&ActiveHousingEstate>,
    owner_content_id: u64,
) -> Option<HousingEstate> {
    active_estate
        .and_then(|estate| database.housing_estate_by_house_id(estate.house_id))
        .filter(|estate| can_edit_housing_estate(estate, owner_content_id))
        .or_else(|| {
            let owned_estates = database.owned_housing_estates(owner_content_id);
            owned_estates
                .iter()
                .find(|estate| !estate.is_apartment && estate.room_number == 0)
                .cloned()
                .or_else(|| owned_estates.into_iter().next())
        })
}

fn resolve_active_indoor_housing_estate(
    database: &mut WorldDatabase,
    active_estate: Option<&ActiveHousingEstate>,
    preferred_apartment_context: Option<ActiveHousingWardContext>,
    zone_id: u16,
    world_id: u16,
    owner_content_id: u64,
) -> Option<ActiveHousingEstate> {
    if let Some(active_estate) = active_estate.filter(|estate| estate.indoors) {
        let estate = database
            .housing_estate_by_house_id(active_estate.house_id)
            .filter(|estate| can_edit_housing_estate(estate, owner_content_id))?;

        return Some(active_housing_estate(&estate, true));
    }

    if apartment_interior_zone_id(zone_id) {
        let owned_estates = database.owned_housing_estates(owner_content_id);
        if let Some(apartment) = preferred_apartment_context
            .and_then(|context| {
                owned_estates
                    .iter()
                    .find(|estate| {
                        estate.is_apartment
                            && estate.room_number > 0
                            && estate.territory_type_id == context.territory_type_id as i32
                            && estate.world_id == world_id as i32
                            && estate.ward_index == context.ward_index as i32
                            && estate.division == context.division as i32
                    })
                    .cloned()
            })
            .or_else(|| {
                owned_estates
                    .into_iter()
                    .find(|estate| estate.is_apartment && estate.room_number > 0)
            })
        {
            return Some(active_housing_estate(&apartment, true));
        }
    }

    selected_or_owned_housing_estate(database, active_estate, owner_content_id)
        .map(|estate| active_housing_estate(&estate, true))
}

fn apartment_interior_zone_id(zone_id: u16) -> bool {
    APARTMENT_INTERIOR_TERRITORY_TYPE_IDS.contains(&zone_id)
}

fn housing_estate_plot_size(estate: &HousingEstate) -> PlotSize {
    PlotSize::from_repr(estate.plot_size as u8).unwrap_or(PlotSize::Large)
}

fn simple_housing_indoor_territory_type_id(plot_size: PlotSize) -> u16 {
    match plot_size {
        PlotSize::Small => DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_SMALL,
        PlotSize::Medium => DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_MEDIUM,
        PlotSize::Large => DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_LARGE,
    }
}

fn housing_default_indoor_entry_territory_type_id_for_estate(estate: &HousingEstate) -> u16 {
    let outdoor_territory_type_id = estate.territory_type_id.clamp(0, u16::MAX as i32) as u16;
    district_default_indoor_territory_type_id(
        outdoor_territory_type_id,
        housing_estate_plot_size(estate),
    )
    .unwrap_or_else(|| simple_housing_indoor_territory_type_id(housing_estate_plot_size(estate)))
}

fn district_default_indoor_territory_type_id(
    outdoor_territory_type_id: u16,
    plot_size: PlotSize,
) -> Option<u16> {
    match (outdoor_territory_type_id, plot_size) {
        (339, PlotSize::Small) => Some(282),
        (339, PlotSize::Medium) => Some(283),
        (339, PlotSize::Large) => Some(284),
        (340, PlotSize::Small) => Some(342),
        (340, PlotSize::Medium) => Some(343),
        (340, PlotSize::Large) => Some(344),
        (341, PlotSize::Small) => Some(345),
        (341, PlotSize::Medium) => Some(346),
        (341, PlotSize::Large) => Some(347),
        (641, PlotSize::Small) => Some(649),
        (641, PlotSize::Medium) => Some(650),
        (641, PlotSize::Large) => Some(651),
        (979, PlotSize::Small) => Some(980),
        (979, PlotSize::Medium) => Some(981),
        (979, PlotSize::Large) => Some(982),
        _ => None,
    }
}

fn active_housing_ward_context_from_estate(estate: &HousingEstate) -> ActiveHousingWardContext {
    ActiveHousingWardContext {
        territory_type_id: estate.territory_type_id.clamp(0, u16::MAX as i32) as u16,
        ward_index: estate.ward_index.clamp(0, u8::MAX as i32) as u8,
        division: estate.division.clamp(0, u8::MAX as i32) as u8,
    }
}

fn display_housing_ward_context_or_default(
    display_context: Option<ActiveHousingWardContext>,
    active_context: Option<ActiveHousingWardContext>,
    active_estate_context: Option<ActiveHousingWardContext>,
    default_context: ActiveHousingWardContext,
) -> ActiveHousingWardContext {
    display_context
        .or(active_context)
        .or(active_estate_context)
        .unwrap_or(default_context)
}

fn placard_authoritative_estate(
    estate: &HousingEstate,
    content_id: u64,
) -> Option<ActiveHousingEstate> {
    if !can_edit_housing_estate(estate, content_id) {
        return None;
    }

    Some(ActiveHousingEstate {
        land_ident: estate.land_ident,
        house_id: outdoor_house_id_from_estate(estate),
        indoors: false,
    })
}

fn outdoor_init_display_context(
    active_context: Option<ActiveHousingWardContext>,
    display_context: Option<ActiveHousingWardContext>,
    default_context: ActiveHousingWardContext,
    zone_id: u16,
) -> ActiveHousingWardContext {
    let context = match active_context {
        Some(context) if context.territory_type_id == zone_id => context,
        _ => display_context.unwrap_or(default_context),
    };

    ActiveHousingWardContext {
        territory_type_id: zone_id,
        ..context
    }
}

fn outdoor_init_active_context(
    active_context: Option<ActiveHousingWardContext>,
    _active_estate_context: Option<ActiveHousingWardContext>,
) -> Option<ActiveHousingWardContext> {
    active_context
}

#[cfg(test)]
fn trusted_housing_ward_context_after_display_update(
    active_context: Option<ActiveHousingWardContext>,
    _display_context: ActiveHousingWardContext,
) -> Option<ActiveHousingWardContext> {
    active_context
}

#[cfg(test)]
fn trusted_housing_ward_context_after_vacant_placard(
    active_context: Option<ActiveHousingWardContext>,
    _display_context: ActiveHousingWardContext,
) -> Option<ActiveHousingWardContext> {
    active_context
}

fn outdoor_init_authoritative_context(
    active_context: Option<ActiveHousingWardContext>,
    _display_context: ActiveHousingWardContext,
    zone_id: u16,
) -> Option<ActiveHousingWardContext> {
    let context = active_context?;
    if context.territory_type_id != zone_id {
        return None;
    }

    Some(context)
}

fn resolve_active_housing_estate_for_outdoor_owner_gate(
    database: &mut WorldDatabase,
    active_housing_ward_context: Option<ActiveHousingWardContext>,
    world_id: u16,
    active_estate: &ActiveHousingEstate,
    content_id: u64,
) -> Option<(ActiveHousingEstate, OutdoorHousingLocation)> {
    if active_estate.indoors || active_estate.house_id.unit.apartment_flag {
        return None;
    }

    let location = outdoor_housing_location_from_raw_entry(
        active_housing_ward_context?,
        active_estate.house_id.unit.apartment_division_plot_index,
    )?;
    let estate = database.housing_estate_by_location(
        location.territory_type_id,
        world_id,
        location.ward_index,
        location.division,
        location.plot_index,
    )?;
    let active_estate = active_housing_estate_for_outdoor_row(&estate, location, content_id)?;

    Some((active_estate, location))
}

fn resolve_active_housing_estate_for_outdoor_edit(
    database: &mut WorldDatabase,
    active_housing_ward_context: Option<ActiveHousingWardContext>,
    world_id: u16,
    raw_plot_index: u8,
    content_id: u64,
) -> Option<(ActiveHousingEstate, OutdoorHousingLocation)> {
    let location =
        outdoor_housing_location_from_raw_entry(active_housing_ward_context?, raw_plot_index)?;
    let estate = database.housing_estate_by_location(
        location.territory_type_id,
        world_id,
        location.ward_index,
        location.division,
        location.plot_index,
    )?;
    let active_estate = active_housing_estate_for_outdoor_row(&estate, location, content_id)?;

    Some((active_estate, location))
}

fn outdoor_housing_location_from_raw_entry(
    context: ActiveHousingWardContext,
    raw_plot_index: u8,
) -> Option<OutdoorHousingLocation> {
    let (division, plot_index) = match raw_plot_index {
        0..=29 => (0, raw_plot_index),
        30..=59 => (1, raw_plot_index - HOUSING_PLOTS_PER_DIVISION),
        _ => return None,
    };

    Some(OutdoorHousingLocation {
        territory_type_id: context.territory_type_id,
        ward_index: context.ward_index,
        division,
        plot_index,
        raw_plot_index,
    })
}

fn active_housing_estate_for_outdoor_row(
    estate: &HousingEstate,
    location: OutdoorHousingLocation,
    content_id: u64,
) -> Option<ActiveHousingEstate> {
    if !housing_estate_matches_outdoor_location(estate, location) {
        return None;
    }
    if !can_edit_housing_estate(estate, content_id) {
        return None;
    }

    Some(ActiveHousingEstate {
        land_ident: estate.land_ident,
        house_id: outdoor_house_id_from_estate_location(estate, location),
        indoors: false,
    })
}

fn active_housing_estate_for_owned_outdoor_pattern(
    database: &mut WorldDatabase,
    active_estate: Option<&ActiveHousingEstate>,
    content_id: u64,
) -> Option<ActiveHousingEstate> {
    selected_or_owned_housing_estate(database, active_estate, content_id)
        .filter(|estate| !estate.is_apartment && estate.room_number == 0)
        .map(|estate| active_housing_estate(&estate, false))
}

fn active_housing_estate_for_outdoor_removal_row(
    active_estate: Option<&ActiveHousingEstate>,
    estate: Option<&HousingEstate>,
    content_id: u64,
) -> Option<ActiveHousingEstate> {
    let active_estate = active_estate?;
    if active_estate.indoors || active_estate.house_id.unit.apartment_flag {
        return None;
    }

    let estate = estate?;
    if active_estate.land_ident != estate.land_ident {
        return None;
    }
    if estate.is_apartment || estate.room_number != 0 {
        return None;
    }
    if !can_edit_housing_estate(estate, content_id) {
        return None;
    }

    Some(ActiveHousingEstate {
        land_ident: estate.land_ident,
        house_id: outdoor_house_id_from_estate(estate),
        indoors: false,
    })
}

fn housing_estate_matches_outdoor_location(
    estate: &HousingEstate,
    location: OutdoorHousingLocation,
) -> bool {
    estate.territory_type_id == location.territory_type_id as i32
        && estate.ward_index == location.ward_index as i32
        && estate.division == location.division as i32
        && estate.plot_index == location.plot_index as i32
        && !estate.is_apartment
        && estate.room_number == 0
}

fn outdoor_housing_location_from_estate(estate: &HousingEstate) -> Option<OutdoorHousingLocation> {
    let territory_type_id = u16::try_from(estate.territory_type_id).ok()?;
    let ward_index = u8::try_from(estate.ward_index).ok()?;
    let division = u8::try_from(estate.division).ok()?;
    let plot_index = u8::try_from(estate.plot_index).ok()?;
    if division > 1 || plot_index >= HOUSING_PLOTS_PER_DIVISION {
        return None;
    }

    Some(OutdoorHousingLocation {
        territory_type_id,
        ward_index,
        division,
        plot_index,
        raw_plot_index: plot_index + (division * HOUSING_PLOTS_PER_DIVISION),
    })
}

fn outdoor_house_id_from_estate(estate: &HousingEstate) -> HouseId {
    let Some(location) = outdoor_housing_location_from_estate(estate) else {
        return HouseId::from_u64(estate.house_id as u64);
    };

    outdoor_house_id_from_estate_location(estate, location)
}

fn outdoor_house_id_from_estate_location(
    estate: &HousingEstate,
    location: OutdoorHousingLocation,
) -> HouseId {
    let persisted = HouseId::from_u64(estate.house_id as u64);

    HouseId {
        unit: HouseUnit {
            apartment_division_plot_index: location.raw_plot_index,
            apartment_flag: false,
        },
        unk1: persisted.unk1,
        ward_index: location.ward_index.min(0x3F),
        room_number: 0,
        territory_type_id: location.territory_type_id,
        world_id: estate.world_id.clamp(0, u16::MAX as i32) as u16,
    }
}

fn initial_housing_furniture_list_count(indoors: bool, list_count: usize) -> usize {
    if indoors {
        list_count.min(INDOOR_FURNITURE_LISTS_BEFORE_FINISH_LOADING)
    } else {
        list_count
    }
}

fn deferred_housing_furniture_list_start_index(indoors: bool, list_count: usize) -> usize {
    initial_housing_furniture_list_count(indoors, list_count)
}

fn build_estate_furniture_lists(
    estate: &HousingEstate,
    rows: &[HousingFurniture],
    indoors: bool,
) -> Vec<FurnitureList> {
    let house_id = if indoors {
        HouseId::from_u64(estate.house_id as u64)
    } else {
        outdoor_house_id_from_estate(estate)
    };

    build_furniture_lists(house_id, rows, indoors, None)
}

fn is_interior_placed_container(container: ContainerType) -> bool {
    interior_placed_container_index(container).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HousingAppearanceSlotSpec {
    container: ContainerType,
    slot: u16,
    additional_data: u32,
    item_ui_category: u8,
    stain: u8,
}

fn populate_housing_appearance_inventory(
    inventory: &mut HousingInventory,
    game_data: &mut GameData,
    estate: &HousingEstate,
) {
    let exterior = house_exterior_from_json(&estate.exterior_json);
    let light_level = estate.light_level.clamp(0, u8::MAX as i32) as u8;
    let interior =
        housing_interior_details_from_json(&estate.interior_json, light_level, estate.is_apartment);

    for spec in housing_exterior_appearance_slot_specs(&exterior)
        .into_iter()
        .chain(housing_interior_appearance_slot_specs(
            &interior,
            estate.is_apartment,
        ))
    {
        populate_housing_appearance_slot(inventory, game_data, spec);
    }
}

fn populate_housing_appearance_slot(
    inventory: &mut HousingInventory,
    game_data: &mut GameData,
    spec: HousingAppearanceSlotSpec,
) {
    let Some(item_id) =
        game_data.get_item_id_by_additional_data(spec.additional_data, spec.item_ui_category)
    else {
        tracing::warn!(
            ?spec.container,
            slot = spec.slot,
            additional_data = spec.additional_data,
            item_ui_category = spec.item_ui_category,
            "Unable to resolve housing appearance fixture item"
        );
        return;
    };

    let Some(slot) = inventory.get_item_mut(spec.container, spec.slot) else {
        tracing::warn!(
            ?spec.container,
            slot = spec.slot,
            item_id,
            "Unable to populate housing appearance slot"
        );
        return;
    };

    *slot = Item {
        quantity: 1,
        item_id,
        condition: ITEM_CONDITION_MAX,
        stains: [spec.stain, 0],
        ..Default::default()
    };
}

fn housing_exterior_appearance_slot_specs(
    exterior: &HouseExterior,
) -> Vec<HousingAppearanceSlotSpec> {
    let container = ContainerType::HousingExteriorAppearance;
    vec![
        HousingAppearanceSlotSpec {
            container,
            slot: 1,
            additional_data: exterior.roof_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_ROOF,
            stain: exterior.colors.roof,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 2,
            additional_data: exterior.walls_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_EXTERIOR_WALL,
            stain: exterior.colors.walls,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 3,
            additional_data: exterior.windows_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_WINDOW,
            stain: exterior.colors.windows,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 4,
            additional_data: exterior.door_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_DOOR,
            stain: exterior.colors.door,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 5,
            additional_data: exterior.roof_fixture_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_ROOF_DECORATION,
            stain: exterior.colors.roof_fixture,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 6,
            additional_data: exterior.wall_fixture_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION,
            stain: exterior.colors.wall_fixture,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 7,
            additional_data: exterior.above_door_banner_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_PLACARD,
            stain: exterior.colors.above_door_banner,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 8,
            additional_data: exterior.fence_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_FENCE,
            stain: exterior.colors.fence,
        },
    ]
    .into_iter()
    .filter(|spec| spec.additional_data != 0)
    .collect()
}

fn housing_interior_appearance_slot_specs(
    interior: &HousingInteriorDetails,
    is_apartment: bool,
) -> Vec<HousingAppearanceSlotSpec> {
    let container = ContainerType::HousingInteriorAppearance;
    let mut specs = vec![
        HousingAppearanceSlotSpec {
            container,
            slot: 0,
            additional_data: interior.ground_walls,
            item_ui_category: ITEM_UI_CATEGORY_INTERIOR_WALL,
            stain: 0,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 1,
            additional_data: interior.ground_floor,
            item_ui_category: ITEM_UI_CATEGORY_FLOORING,
            stain: 0,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 2,
            additional_data: interior.ground_chandelier,
            item_ui_category: ITEM_UI_CATEGORY_CEILING_LIGHT,
            stain: 0,
        },
    ];

    if !is_apartment {
        specs.extend([
            HousingAppearanceSlotSpec {
                container,
                slot: 3,
                additional_data: interior.top_walls,
                item_ui_category: ITEM_UI_CATEGORY_INTERIOR_WALL,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 4,
                additional_data: interior.top_floor,
                item_ui_category: ITEM_UI_CATEGORY_FLOORING,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 5,
                additional_data: interior.top_chandelier,
                item_ui_category: ITEM_UI_CATEGORY_CEILING_LIGHT,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 6,
                additional_data: interior.cellar_walls,
                item_ui_category: ITEM_UI_CATEGORY_INTERIOR_WALL,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 7,
                additional_data: interior.cellar_floor,
                item_ui_category: ITEM_UI_CATEGORY_FLOORING,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 8,
                additional_data: interior.cellar_chandelier,
                item_ui_category: ITEM_UI_CATEGORY_CEILING_LIGHT,
                stain: 0,
            },
        ]);
    }

    specs
        .into_iter()
        .filter(|spec| spec.additional_data != 0)
        .collect()
}

fn clear_housing_reset_cache(
    active_housing_estate: &mut Option<ActiveHousingEstate>,
    house_inventory: &mut HousingInventory,
    clear_active_estate: bool,
) {
    *house_inventory = HousingInventory::default();
    if clear_active_estate {
        *active_housing_estate = None;
    }
}

fn placed_container_for_flat_slot(flat_slot: u16, indoors: bool) -> Option<(ContainerType, u16)> {
    if indoors {
        return indoor_container_for_flat_slot(flat_slot);
    }

    if flat_slot < 50 {
        Some((ContainerType::HousingExteriorPlacedItems, flat_slot))
    } else {
        None
    }
}

fn housing_indoor_entry_transform(is_apartment: bool) -> HousingEntryTransform {
    if is_apartment {
        return HousingEntryTransform {
            position: Position(Vec3::ZERO),
            rotation: 0.0,
        };
    }

    HousingEntryTransform {
        position: Position(Vec3::new(-0.25, -0.39, 5.0)),
        rotation: 180.0,
    }
}

fn housing_interior_pattern_area_allows_request(intended_use: TerritoryIntendedUse) -> bool {
    matches!(
        intended_use,
        TerritoryIntendedUse::HousingOutdoor | TerritoryIntendedUse::HousingIndoor
    )
}

fn housing_interior_pattern_apply_should_reload(intended_use: TerritoryIntendedUse) -> bool {
    intended_use == TerritoryIntendedUse::HousingIndoor
}

fn can_edit_housing_estate(estate: &HousingEstate, content_id: u64) -> bool {
    estate.owner_content_id == Some(content_id as i64)
}

fn should_broadcast_housing_item_removal(container: ContainerType) -> bool {
    container == ContainerType::HousingExteriorPlacedItems
        || is_interior_placed_container(container)
}

fn is_housing_appearance_container(container: ContainerType) -> bool {
    matches!(
        container,
        ContainerType::HousingExteriorAppearance
            | ContainerType::HousingExteriorAppearanceEdit
            | ContainerType::HousingInteriorAppearance
            | ContainerType::HousingInteriorAppearanceEdit
    )
}

fn housing_appearance_container_intended_use(
    container: ContainerType,
) -> Option<TerritoryIntendedUse> {
    match container {
        ContainerType::HousingExteriorAppearance | ContainerType::HousingExteriorAppearanceEdit => {
            Some(TerritoryIntendedUse::HousingOutdoor)
        }
        ContainerType::HousingInteriorAppearance | ContainerType::HousingInteriorAppearanceEdit => {
            Some(TerritoryIntendedUse::HousingIndoor)
        }
        _ => None,
    }
}

fn housing_appearance_operation_involves_staging(action: &ItemOperation) -> bool {
    is_housing_appearance_container(action.src_storage_id)
        || is_housing_appearance_container(action.dst_storage_id)
}

fn housing_appearance_operation_matches_area(
    action: &ItemOperation,
    intended_use: TerritoryIntendedUse,
) -> bool {
    [action.src_storage_id, action.dst_storage_id]
        .into_iter()
        .filter_map(housing_appearance_container_intended_use)
        .all(|container_use| container_use == intended_use)
}

fn default_housing_appearance_target_slot(
    item_ui_category: u8,
    intended_use: TerritoryIntendedUse,
    is_apartment: bool,
) -> Option<u16> {
    match intended_use {
        TerritoryIntendedUse::HousingIndoor => {
            let base_slot = if is_apartment { 0 } else { 3 };
            match item_ui_category {
                ITEM_UI_CATEGORY_INTERIOR_WALL => Some(base_slot),
                ITEM_UI_CATEGORY_FLOORING => Some(base_slot + 1),
                ITEM_UI_CATEGORY_CEILING_LIGHT => Some(base_slot + 2),
                _ => None,
            }
        }
        TerritoryIntendedUse::HousingOutdoor => match item_ui_category {
            ITEM_UI_CATEGORY_ROOF => Some(1),
            ITEM_UI_CATEGORY_EXTERIOR_WALL => Some(2),
            ITEM_UI_CATEGORY_WINDOW => Some(3),
            ITEM_UI_CATEGORY_DOOR => Some(4),
            ITEM_UI_CATEGORY_ROOF_DECORATION => Some(5),
            ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION => Some(6),
            ITEM_UI_CATEGORY_PLACARD => Some(7),
            ITEM_UI_CATEGORY_FENCE => Some(8),
            _ => None,
        },
        _ => None,
    }
}

fn housing_appearance_marker_target_slot(
    item_ui_category: u8,
    intended_use: TerritoryIntendedUse,
    is_apartment: bool,
    marker_target_slot: Option<u16>,
) -> Option<u16> {
    let target_slot = marker_target_slot?;
    if intended_use == TerritoryIntendedUse::HousingIndoor && is_apartment && target_slot > 2 {
        return None;
    }

    let expected_category = match intended_use {
        TerritoryIntendedUse::HousingIndoor => {
            housing_interior_item_ui_category_for_slot(target_slot)
        }
        TerritoryIntendedUse::HousingOutdoor => {
            housing_exterior_item_ui_category_for_slot(target_slot)
        }
        _ => None,
    };

    (expected_category == Some(item_ui_category)).then_some(target_slot)
}

fn housing_appearance_item_data(game_data: &mut GameData, item: Item) -> Option<(u32, u8)> {
    if item.is_empty_slot() {
        return None;
    }

    let row = game_data.get_item_info(ItemInfoQuery::ById(item.item_id))?;
    (row.additional_data != 0).then_some((row.additional_data, row.item_ui_category))
}

fn housing_interior_field_for_appearance_slot(slot: u16) -> Option<HousingInteriorField> {
    match slot {
        0 => Some(HousingInteriorField::GroundWalls),
        1 => Some(HousingInteriorField::GroundFloor),
        2 => Some(HousingInteriorField::GroundChandelier),
        3 => Some(HousingInteriorField::TopWalls),
        4 => Some(HousingInteriorField::TopFloor),
        5 => Some(HousingInteriorField::TopChandelier),
        6 => Some(HousingInteriorField::CellarWalls),
        7 => Some(HousingInteriorField::CellarFloor),
        8 => Some(HousingInteriorField::CellarChandelier),
        _ => None,
    }
}

fn housing_interior_item_ui_category_for_slot(slot: u16) -> Option<u8> {
    match slot % 3 {
        0 => Some(ITEM_UI_CATEGORY_INTERIOR_WALL),
        1 => Some(ITEM_UI_CATEGORY_FLOORING),
        2 => Some(ITEM_UI_CATEGORY_CEILING_LIGHT),
        _ => None,
    }
}

fn housing_exterior_field_for_appearance_slot(slot: u16) -> Option<HousingExteriorField> {
    match slot {
        1 => Some(HousingExteriorField::Roof),
        2 => Some(HousingExteriorField::Walls),
        3 => Some(HousingExteriorField::Windows),
        4 => Some(HousingExteriorField::Door),
        5 => Some(HousingExteriorField::RoofFixture),
        6 => Some(HousingExteriorField::WallFixture),
        7 => Some(HousingExteriorField::AboveDoorBanner),
        8 => Some(HousingExteriorField::Fence),
        _ => None,
    }
}

fn housing_exterior_item_ui_category_for_slot(slot: u16) -> Option<u8> {
    match slot {
        1 => Some(ITEM_UI_CATEGORY_ROOF),
        2 => Some(ITEM_UI_CATEGORY_EXTERIOR_WALL),
        3 => Some(ITEM_UI_CATEGORY_WINDOW),
        4 => Some(ITEM_UI_CATEGORY_DOOR),
        5 => Some(ITEM_UI_CATEGORY_ROOF_DECORATION),
        6 => Some(ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION),
        7 => Some(ITEM_UI_CATEGORY_PLACARD),
        8 => Some(ITEM_UI_CATEGORY_FENCE),
        _ => None,
    }
}

fn housing_exterior_color_field_for_appearance_slot(
    slot: u16,
) -> Option<HousingExteriorColorField> {
    match slot {
        1 => Some(HousingExteriorColorField::Roof),
        2 => Some(HousingExteriorColorField::Walls),
        3 => Some(HousingExteriorColorField::Windows),
        4 => Some(HousingExteriorColorField::Door),
        5 => Some(HousingExteriorColorField::RoofFixture),
        6 => Some(HousingExteriorColorField::WallFixture),
        7 => Some(HousingExteriorColorField::AboveDoorBanner),
        8 => Some(HousingExteriorColorField::Fence),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::init::{
        build_outdoor_estate_furniture_lists, housing_furniture_object_from_row,
        housing_furniture_objects_from_rows, housing_indoor_init_needs_resolution,
        housing_interior_ready_primary_id, housing_object_data_value_sets_from_rows,
        is_interior_placed_furniture_row, should_defer_housing_indoor_finish_loading,
        should_hide_additional_chambers_door, should_sync_indoor_overlays_after_loading,
        should_sync_indoor_overlays_after_remodel, should_sync_indoor_overlays_on_finish_zoning,
    };
    use super::*;
    use crate::{DEFAULT_LOCAL_HOUSING_LAND_FLAGS, HousingEstateSpec, WorldDatabase};
    use kawari::common::HouseUnit;
    use kawari::{
        common::HousingFlag,
        ipc::zone::{
            AvailabilityType, HouseExterior, HouseExteriorColors, HouseStatus, HousingFlags,
            PlotSize, PurchaseType, TenantType,
        },
    };

    trait HousingJsonMutationOutcome {
        fn is_error(&self) -> bool;
        fn into_success(self) -> Option<String>;
    }

    impl HousingJsonMutationOutcome for String {
        fn is_error(&self) -> bool {
            false
        }

        fn into_success(self) -> Option<String> {
            Some(self)
        }
    }

    impl HousingJsonMutationOutcome for Result<String, serde_json::Error> {
        fn is_error(&self) -> bool {
            self.is_err()
        }

        fn into_success(self) -> Option<String> {
            self.ok()
        }
    }

    fn house_id(plot_index: u8, room_number: u16, is_apartment: bool) -> HouseId {
        HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: plot_index,
                apartment_flag: is_apartment,
            },
            unk1: 0,
            ward_index: 2,
            room_number,
            territory_type_id: 340,
            world_id: 21,
        }
    }

    fn estate(id: HouseId, flags: i32, is_apartment: bool) -> HousingEstate {
        HousingEstate {
            house_id: id.to_u64() as i64,
            flags,
            is_apartment,
            ..Default::default()
        }
    }

    fn furniture_row(catalog_id: i32, position: Position, rotation: f32) -> HousingFurniture {
        HousingFurniture {
            catalog_id,
            pos_x: position.0.x,
            pos_y: position.0.y,
            pos_z: position.0.z,
            rotation,
            placed: true,
            ..Default::default()
        }
    }

    fn estate_row_for_plot(plot_index: i32, flags: i32, plot_size: PlotSize) -> HousingEstate {
        HousingEstate {
            house_id: house_id(plot_index as u8, 0, false).to_u64() as i64,
            flags,
            plot_index,
            plot_size: plot_size as i32,
            exterior_json: "{}".to_string(),
            ..Default::default()
        }
    }

    fn ward_estate(plot_index: i32, division: i32, flags: i32) -> HousingEstate {
        HousingEstate {
            house_id: house_id(plot_index as u8, 0, false).to_u64() as i64,
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division,
            plot_index,
            owner_content_id: Some(12345),
            owner_name: "Local Owner".to_string(),
            plot_size: PlotSize::Large as i32,
            flags,
            estate_name: "Local Estate".to_string(),
            greeting: "Welcome from the DB.".to_string(),
            exterior_json: "{}".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn build_housing_ward_info_marks_personal_large_plot_owned() {
        let ward_info = build_housing_ward_info(340, 21, 2, &[ward_estate(5, 0, 0x0B)], &[]);

        assert_eq!(ward_info.id.territory_type_id, 340);
        assert_eq!(ward_info.id.world_id, 21);
        assert_eq!(ward_info.id.ward_index, 2);
        assert_eq!(ward_info.house_summaries.len(), 60);
        assert_eq!(ward_info.purchase_type, PurchaseType::Unavailable);
        assert_eq!(ward_info.tenant_type, TenantType::Any);

        let summary = &ward_info.house_summaries[5];
        assert_eq!(summary.name, "Local Estate");
        assert_eq!(
            summary.flags,
            HousingFlags::PLOT_OWNED | HousingFlags::VISITORS_ALLOWED | HousingFlags::HOUSE_BUILT
        );
        assert_eq!(
            ward_info.house_summaries[35].flags,
            HousingFlags::empty(),
            "subdivision slot remains vacant when no subdivision estate exists"
        );
    }

    #[test]
    fn occupied_land_info_uses_character_owner_without_fc_tag() {
        let land_info = housing_occupied_land_info_from_estate(&ward_estate(5, 0, 0x0B));

        assert_eq!(land_info.owner_id, 12345);
        assert_eq!(land_info.owner_name, "Local Owner");
        assert_eq!(land_info.fc_tag, "");
        assert_eq!(land_info.house_size, PlotSize::Large);
        assert_eq!(land_info.house_icon, 1);
        assert_eq!(land_info.estate_name, "Local Estate");
        assert_eq!(land_info.estate_greeting, "Welcome from the DB.");
    }

    #[test]
    fn vacant_land_info_is_unavailable_for_local_server() {
        let land_info = housing_vacant_land_info();

        assert_eq!(land_info.purchase_type, PurchaseType::Unavailable);
        assert_eq!(land_info.availability_type, AvailabilityType::Unavailable);
        assert_eq!(land_info.tenant_type, TenantType::Any);
        assert_eq!(&land_info.unk4[..12], &[0; 12]);
        assert_eq!(&land_info.unk4[12..], &[0xFF; 8]);
    }

    #[test]
    fn estate_greeting_uses_db_greeting() {
        let estate = ward_estate(5, 0, 0x0B);

        let greeting = housing_estate_greeting_from_estate(&estate);

        assert_eq!(greeting.id, HouseId::from_u64(estate.house_id as u64));
        assert_eq!(greeting.greeting, "Welcome from the DB.");
    }

    #[test]
    fn estate_land_data_uses_packed_house_id_and_flags() {
        let id = house_id(4, 0, false);

        let land_data = estate_land_data(&estate(id, 1, false));

        assert_eq!(land_data.id, id);
        assert_eq!(land_data.flags, 1);
    }

    #[test]
    fn owned_housing_land_data_routes_retail_owned_housing_slots() {
        let owned_house_id = house_id(4, 0, false);
        let fc_house_id = house_id(6, 0, false);
        let apartment_id = house_id(0, 1, true);
        let mut fc_estate = estate(fc_house_id, 1, false);
        fc_estate.flags |= FREE_COMPANY_HOUSING_FLAG;

        let (owned, apartment) = owned_housing_land_data(&[
            estate(owned_house_id, 1, false),
            fc_estate,
            estate(apartment_id, 19, true),
        ]);

        assert_eq!(owned[0].id, fc_house_id);
        assert_eq!(owned[1].id, owned_house_id);
        assert_eq!(owned[2].id, HouseId::default());
        assert_eq!(owned[3].id, HouseId::default());
        assert_eq!(owned[4].id, HouseId::default());
        assert_eq!(apartment.id, apartment_id);
    }

    #[test]
    fn build_apartment_list_entries_uses_db_residents() {
        let room_2 = HousingEstate {
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 0,
            room_number: 2,
            is_apartment: true,
            owner_name: "Room Two".to_string(),
            greeting: "Second room greeting".to_string(),
            flags: DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
            ..Default::default()
        };
        let room_3 = HousingEstate {
            territory_type_id: 341,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 0,
            room_number: 3,
            is_apartment: true,
            owner_name: "Room Three".to_string(),
            greeting: "Third room greeting".to_string(),
            flags: DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
            ..Default::default()
        };

        let entries = build_apartment_list_entries(&[room_2, room_3], 1);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].resident_name, "Room Two");
        assert_eq!(entries[0].resident_zone_id, 340);
        assert_eq!(entries[0].apartment_description, "Second room greeting");
        assert!(entries[0].visitors_permitted);
        assert_eq!(entries[1].resident_name, "Room Three");
        assert_eq!(entries[1].resident_zone_id, 341);
    }

    #[test]
    fn build_furniture_lists_empty_indoor_sends_one_empty_list() {
        let id = house_id(4, 0, false);

        let lists = build_furniture_lists(id, &[], true, None);

        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, id);
        assert_eq!(lists[0].count, 1);
        assert_eq!(lists[0].index, 0);
        assert_eq!(lists[0].unk2, 100);
        assert!(lists[0].furniture.is_empty());
    }

    #[test]
    fn build_furniture_lists_maps_single_furniture_row() {
        let id = house_id(4, 0, false);
        let position = Position(Vec3::new(1.0, 2.0, 3.0));

        let lists = build_furniture_lists(id, &[furniture_row(123, position, 1.25)], true, None);

        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].furniture.len(), 1);
        assert_eq!(lists[0].furniture[0].id, 123);
        assert_eq!(lists[0].furniture[0].position, position);
        assert_eq!(lists[0].furniture[0].rotation, 1.25);
    }

    #[test]
    fn build_furniture_lists_chunks_after_one_hundred_rows() {
        let id = house_id(4, 0, false);
        let rows = vec![furniture_row(123, Position::default(), 0.0); 101];

        let lists = build_furniture_lists(id, &rows, true, None);

        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].count, 2);
        assert_eq!(lists[0].index, 0);
        assert_eq!(lists[0].furniture.len(), 100);
        assert_eq!(lists[1].count, 2);
        assert_eq!(lists[1].index, 1);
        assert_eq!(lists[1].furniture.len(), 1);
    }

    #[test]
    fn build_furniture_lists_uses_indoor_slot_capacity() {
        let id = house_id(4, 0, false);
        let rows = vec![furniture_row(123, Position::default(), 0.0); 300];

        let lists = build_furniture_lists(id, &rows, true, Some(450));

        assert_eq!(lists.len(), 5);
        assert!(lists.iter().all(|list| list.count == 5));
        assert_eq!(lists[0].unk2, 100);
        assert_eq!(lists[3].unk2, 100);
        assert_eq!(lists[4].unk2, 50);
        assert_eq!(lists[0].furniture.len(), 100);
        assert_eq!(lists[1].furniture.len(), 100);
        assert_eq!(lists[2].furniture.len(), 100);
        assert_eq!(lists[3].furniture.len(), 0);
        assert_eq!(lists[4].furniture.len(), 0);
    }

    #[test]
    fn build_furniture_lists_large_capacity_matches_six_hundred_slots() {
        let id = house_id(4, 0, false);
        let rows = vec![furniture_row(123, Position::default(), 0.0); 598];

        let lists = build_furniture_lists(id, &rows, true, Some(600));

        assert_eq!(lists.len(), 6);
        assert!(lists.iter().all(|list| list.count == 6));
        assert!(lists.iter().all(|list| list.unk2 == 100));
        assert_eq!(lists[0].furniture.len(), 100);
        assert_eq!(lists[5].furniture.len(), 98);
    }

    #[test]
    fn indoor_furniture_list_tail_is_deferred_until_remodel_gate() {
        assert_eq!(initial_housing_furniture_list_count(true, 6), 3);
        assert_eq!(deferred_housing_furniture_list_start_index(true, 6), 3);
        assert_eq!(initial_housing_furniture_list_count(true, 3), 3);
        assert_eq!(deferred_housing_furniture_list_start_index(true, 3), 3);
        assert_eq!(initial_housing_furniture_list_count(false, 8), 8);
        assert_eq!(deferred_housing_furniture_list_start_index(false, 8), 8);
    }

    #[test]
    fn housing_interior_ready_primary_id_matches_retail_halves() {
        let id = HouseId::from_u64(0x003d_0153_0014_0001);

        assert_eq!(housing_interior_ready_primary_id(id), 0x0014_0001_003d_0153);
    }

    #[test]
    fn finish_loading_defers_only_for_pending_indoor_tail() {
        assert!(should_defer_housing_indoor_finish_loading(
            TerritoryIntendedUse::HousingIndoor,
            true
        ));
        assert!(!should_defer_housing_indoor_finish_loading(
            TerritoryIntendedUse::HousingIndoor,
            false
        ));
        assert!(!should_defer_housing_indoor_finish_loading(
            TerritoryIntendedUse::HousingOutdoor,
            true
        ));
    }

    #[test]
    fn housing_object_data_value_sets_use_flat_indoor_furniture_slots() {
        let mut first = furniture_row(111, Position::default(), 0.0);
        first.container_type = ContainerType::HousingInteriorPlacedItems1 as i32;
        first.slot = 4;

        let mut second_page = furniture_row(222, Position::default(), 0.0);
        second_page.container_type = ContainerType::HousingInteriorPlacedItems2 as i32;
        second_page.slot = 2;

        let value_sets = housing_object_data_value_sets_from_rows(&[first, second_page]);

        assert_eq!(value_sets.len(), 2);
        assert_eq!(value_sets[0].furniture_index, 4);
        assert_eq!(value_sets[1].furniture_index, 52);
        assert!(
            value_sets
                .iter()
                .all(|value_set| value_set.value_count == 0)
        );
    }

    #[test]
    fn indoor_furniture_object_overlays_sync_before_finish_zoning() {
        assert!(should_sync_indoor_overlays_after_remodel(
            TerritoryIntendedUse::HousingIndoor,
            true,
            true
        ));
        assert!(!should_sync_indoor_overlays_after_remodel(
            TerritoryIntendedUse::HousingIndoor,
            false,
            true
        ));
        assert!(!should_sync_indoor_overlays_after_remodel(
            TerritoryIntendedUse::HousingIndoor,
            true,
            false
        ));
        assert!(should_sync_indoor_overlays_after_loading(
            TerritoryIntendedUse::HousingIndoor,
            false,
            true
        ));
        assert!(!should_sync_indoor_overlays_after_loading(
            TerritoryIntendedUse::HousingIndoor,
            true,
            true
        ));
        assert!(!should_sync_indoor_overlays_after_remodel(
            TerritoryIntendedUse::HousingOutdoor,
            true,
            true
        ));
        assert!(!should_sync_indoor_overlays_on_finish_zoning(
            TerritoryIntendedUse::HousingIndoor,
            false
        ));
        assert!(!should_sync_indoor_overlays_on_finish_zoning(
            TerritoryIntendedUse::HousingIndoor,
            true
        ));
        assert!(!should_sync_indoor_overlays_on_finish_zoning(
            TerritoryIntendedUse::HousingOutdoor,
            true
        ));
    }

    #[test]
    fn outdoor_furniture_object_overlays_do_not_use_indoor_loading_gate() {
        assert!(!should_sync_indoor_overlays_after_loading(
            TerritoryIntendedUse::HousingOutdoor,
            false,
            true
        ));
        assert!(!should_sync_indoor_overlays_on_finish_zoning(
            TerritoryIntendedUse::HousingIndoor,
            false
        ));
    }

    #[test]
    fn build_furniture_lists_marks_outdoor_lists() {
        let id = house_id(4, 0, false);

        let lists = build_furniture_lists(id, &[], false, None);

        assert_eq!(lists[0].unk2, 0);
    }

    #[test]
    fn interior_placed_furniture_row_filter_rejects_exterior_and_storeroom_rows() {
        let mut indoor = furniture_row(111, Position::default(), 0.0);
        indoor.container_type = ContainerType::HousingInteriorPlacedItems1 as i32;
        assert!(is_interior_placed_furniture_row(&indoor));

        let mut last_indoor_page = furniture_row(222, Position::default(), 0.0);
        last_indoor_page.container_type = ContainerType::HousingInteriorPlacedItems12 as i32;
        assert!(is_interior_placed_furniture_row(&last_indoor_page));

        let mut exterior = furniture_row(333, Position::default(), 0.0);
        exterior.container_type = ContainerType::HousingExteriorPlacedItems as i32;
        assert!(!is_interior_placed_furniture_row(&exterior));

        let mut storeroom = furniture_row(444, Position::default(), 0.0);
        storeroom.container_type = ContainerType::HousingInteriorStoreroom1 as i32;
        assert!(!is_interior_placed_furniture_row(&storeroom));

        let mut unplaced_indoor = indoor;
        unplaced_indoor.placed = false;
        assert!(!is_interior_placed_furniture_row(&unplaced_indoor));
    }

    #[test]
    fn housing_furniture_object_from_row_rejects_rows_from_the_other_area() {
        let mut exterior = furniture_row(321, Position::default(), 0.5);
        exterior.container_type = ContainerType::HousingExteriorPlacedItems as i32;
        exterior.slot = 5;

        assert!(housing_furniture_object_from_row(&exterior, false, 5).is_some());
        assert!(housing_furniture_object_from_row(&exterior, true, 0).is_none());

        let mut interior = furniture_row(654, Position::default(), 1.0);
        interior.container_type = ContainerType::HousingInteriorPlacedItems1 as i32;
        interior.slot = 3;

        assert!(housing_furniture_object_from_row(&interior, true, 0).is_some());
        assert!(housing_furniture_object_from_row(&interior, false, 5).is_none());
    }

    #[test]
    fn housing_furniture_objects_from_rows_keeps_only_matching_placed_area() {
        let mut interior = furniture_row(654, Position::default(), 1.0);
        interior.container_type = ContainerType::HousingInteriorPlacedItems1 as i32;
        interior.slot = 3;

        let mut exterior = furniture_row(321, Position::default(), 0.5);
        exterior.container_type = ContainerType::HousingExteriorPlacedItems as i32;
        exterior.slot = 5;

        let mut storeroom = furniture_row(111, Position::default(), 0.0);
        storeroom.container_type = ContainerType::HousingInteriorStoreroom1 as i32;
        storeroom.slot = 8;

        let mut unplaced = interior.clone();
        unplaced.placed = false;

        let rows = vec![interior, exterior, storeroom, unplaced];
        let indoor_objects = housing_furniture_objects_from_rows(&rows, true, 0);
        let outdoor_objects = housing_furniture_objects_from_rows(&rows, false, 5);

        assert_eq!(indoor_objects.len(), 1);
        assert_eq!(indoor_objects[0].slot, 3);
        assert!(indoor_objects[0].indoors);
        assert_eq!(outdoor_objects.len(), 1);
        assert_eq!(outdoor_objects[0].slot, 5);
        assert!(!outdoor_objects[0].indoors);
        assert_eq!(outdoor_objects[0].plot_index, 5);
    }

    #[test]
    fn build_house_list_houses_maps_db_row_to_plot_slot() {
        let houses = build_house_list_houses(&[estate_row_for_plot(5, 0x0B, PlotSize::Large)]);

        assert_eq!(houses[5].plot_size, PlotSize::Large);
        assert_eq!(houses[5].status, HouseStatus::HouseBuilt);
        assert_eq!(houses[5].flags, HousingFlag::OPEN);
        assert_eq!(houses[5].exterior.roof_id, 1081);
        assert_eq!(houses[5].exterior.walls_id, 3632);
        assert_eq!(houses[5].exterior.windows_id, 2579);
        assert_eq!(houses[5].exterior.door_id, 531);
    }

    #[test]
    fn build_house_list_houses_parses_exterior_json() {
        let mut estate = estate_row_for_plot(2, 0x1B, PlotSize::Medium);
        estate.exterior_json = r#"{
            "roof_id": 1,
            "walls_id": 2,
            "windows_id": 3,
            "door_id": 4,
            "roof_fixture_id": 5,
            "wall_fixture_id": 6,
            "above_door_banner_id": 7,
            "fence_id": 8,
            "colors": {
                "roof": 9,
                "walls": 10,
                "windows": 11,
                "door": 12,
                "roof_fixture": 13,
                "wall_fixture": 14,
                "above_door_banner": 15,
                "fence": 16
            }
        }"#
        .to_string();

        let houses = build_house_list_houses(&[estate]);

        assert_eq!(houses[2].plot_size, PlotSize::Medium);
        assert_eq!(houses[2].status, HouseStatus::HouseBuilt);
        assert_eq!(
            houses[2].flags,
            HousingFlag::OPEN | HousingFlag::OWNED_BY_FC
        );
        assert_eq!(houses[2].exterior.roof_id, 1);
        assert_eq!(houses[2].exterior.walls_id, 2);
        assert_eq!(houses[2].exterior.windows_id, 3);
        assert_eq!(houses[2].exterior.door_id, 4);
        assert_eq!(houses[2].exterior.roof_fixture_id, 5);
        assert_eq!(houses[2].exterior.wall_fixture_id, 6);
        assert_eq!(houses[2].exterior.above_door_banner_id, 7);
        assert_eq!(houses[2].exterior.fence_id, 8);
        assert_eq!(houses[2].exterior.colors.roof, 9);
        assert_eq!(houses[2].exterior.colors.walls, 10);
        assert_eq!(houses[2].exterior.colors.windows, 11);
        assert_eq!(houses[2].exterior.colors.door, 12);
        assert_eq!(houses[2].exterior.colors.roof_fixture, 13);
        assert_eq!(houses[2].exterior.colors.wall_fixture, 14);
        assert_eq!(houses[2].exterior.colors.above_door_banner, 15);
        assert_eq!(houses[2].exterior.colors.fence, 16);
    }

    #[test]
    fn build_house_list_houses_leaves_empty_slots_default() {
        let houses = build_house_list_houses(&[]);
        assert_eq!(houses[11].plot_size, PlotSize::default());
        assert_eq!(houses[11].status, HouseStatus::default());
        assert_eq!(houses[11].flags, HousingFlag::default());
        assert_eq!(houses[11].exterior.roof_id, 0);

        let houses = build_house_list_houses(&[estate_row_for_plot(11, 0x0B, PlotSize::Large)]);
        assert_eq!(houses[11].plot_size, PlotSize::Large);
        assert_eq!(houses[11].flags, HousingFlag::OPEN);
    }

    #[test]
    fn housing_inventory_from_rows_restores_placed_and_storeroom_items() {
        let rows = vec![
            HousingFurniture {
                container_type: ContainerType::HousingInteriorPlacedItems1 as i32,
                slot: 0,
                item_id: 1000,
                stain: 7,
                placed: true,
                ..Default::default()
            },
            HousingFurniture {
                container_type: ContainerType::HousingInteriorStoreroom1 as i32,
                slot: 1,
                item_id: 1001,
                stain: 8,
                placed: false,
                ..Default::default()
            },
        ];

        let inventory = housing_inventory_from_rows(&rows);

        let placed = inventory
            .get_item(ContainerType::HousingInteriorPlacedItems1, 0)
            .unwrap();
        assert_eq!(placed.item_id, 1000);
        assert_eq!(placed.quantity, 1);
        assert_eq!(placed.stains[0], 7);

        let stored = inventory
            .get_item(ContainerType::HousingInteriorStoreroom1, 1)
            .unwrap();
        assert_eq!(stored.item_id, 1001);
        assert_eq!(stored.quantity, 1);
        assert_eq!(stored.stains[0], 8);
    }

    #[test]
    fn housing_inventory_from_rows_ignores_invalid_storage_rows() {
        let rows = vec![
            HousingFurniture {
                container_type: ContainerType::Inventory0 as i32,
                slot: 0,
                item_id: 1000,
                ..Default::default()
            },
            HousingFurniture {
                container_type: ContainerType::HousingInteriorPlacedItems1 as i32,
                slot: housing_container_slot_capacity(ContainerType::HousingInteriorPlacedItems1)
                    as i32,
                item_id: 1001,
                ..Default::default()
            },
        ];

        let inventory = housing_inventory_from_rows(&rows);

        assert!(
            inventory
                .get_item(ContainerType::HousingInteriorPlacedItems1, 0)
                .unwrap()
                .is_empty_slot()
        );
    }

    #[test]
    fn housing_exterior_appearance_slots_follow_sapphire_order() {
        let exterior = HouseExterior {
            roof_id: 11,
            walls_id: 12,
            windows_id: 13,
            door_id: 14,
            roof_fixture_id: 15,
            wall_fixture_id: 16,
            above_door_banner_id: 17,
            fence_id: 18,
            colors: HouseExteriorColors {
                roof: 1,
                walls: 2,
                windows: 3,
                door: 4,
                roof_fixture: 5,
                wall_fixture: 6,
                above_door_banner: 7,
                fence: 8,
            },
        };

        let specs = housing_exterior_appearance_slot_specs(&exterior);

        assert_eq!(specs.len(), 8);
        assert_eq!(specs[0].slot, 1);
        assert_eq!(specs[0].additional_data, 11);
        assert_eq!(specs[0].item_ui_category, ITEM_UI_CATEGORY_ROOF);
        assert_eq!(specs[0].stain, 1);
        assert_eq!(specs[7].slot, 8);
        assert_eq!(specs[7].additional_data, 18);
        assert_eq!(specs[7].item_ui_category, ITEM_UI_CATEGORY_FENCE);
        assert_eq!(specs[7].stain, 8);
    }

    #[test]
    fn housing_interior_appearance_slots_follow_sapphire_order() {
        let details = housing_interior_details(2);

        let house_specs = housing_interior_appearance_slot_specs(&details, false);
        let apartment_specs = housing_interior_appearance_slot_specs(&details, true);

        assert_eq!(house_specs.len(), 9);
        assert_eq!(house_specs[0].slot, 0);
        assert_eq!(house_specs[0].additional_data, MINIMAL_INTERIOR_WALL);
        assert_eq!(
            house_specs[0].item_ui_category,
            ITEM_UI_CATEGORY_INTERIOR_WALL
        );
        assert_eq!(house_specs[1].slot, 1);
        assert_eq!(house_specs[1].item_ui_category, ITEM_UI_CATEGORY_FLOORING);
        assert_eq!(house_specs[2].slot, 2);
        assert_eq!(
            house_specs[2].item_ui_category,
            ITEM_UI_CATEGORY_CEILING_LIGHT
        );
        assert_eq!(house_specs[8].slot, 8);
        assert_eq!(house_specs[8].additional_data, MINIMAL_INTERIOR_LIGHT);
        assert_eq!(apartment_specs.len(), 3);
    }

    #[test]
    fn housing_interior_appearance_slots_map_to_fields_and_categories() {
        let expected = [
            (
                HousingInteriorField::GroundWalls,
                ITEM_UI_CATEGORY_INTERIOR_WALL,
            ),
            (HousingInteriorField::GroundFloor, ITEM_UI_CATEGORY_FLOORING),
            (
                HousingInteriorField::GroundChandelier,
                ITEM_UI_CATEGORY_CEILING_LIGHT,
            ),
            (
                HousingInteriorField::TopWalls,
                ITEM_UI_CATEGORY_INTERIOR_WALL,
            ),
            (HousingInteriorField::TopFloor, ITEM_UI_CATEGORY_FLOORING),
            (
                HousingInteriorField::TopChandelier,
                ITEM_UI_CATEGORY_CEILING_LIGHT,
            ),
            (
                HousingInteriorField::CellarWalls,
                ITEM_UI_CATEGORY_INTERIOR_WALL,
            ),
            (HousingInteriorField::CellarFloor, ITEM_UI_CATEGORY_FLOORING),
            (
                HousingInteriorField::CellarChandelier,
                ITEM_UI_CATEGORY_CEILING_LIGHT,
            ),
        ];

        for (slot, (field, category)) in expected.into_iter().enumerate() {
            let slot = slot as u16;
            assert_eq!(
                housing_interior_field_for_appearance_slot(slot),
                Some(field)
            );
            assert_eq!(
                housing_interior_item_ui_category_for_slot(slot),
                Some(category)
            );
        }

        assert_eq!(housing_interior_field_for_appearance_slot(9), None);
    }

    #[test]
    fn default_house_interior_appearance_target_uses_main_floor_slots() {
        assert_eq!(
            default_housing_appearance_target_slot(
                ITEM_UI_CATEGORY_INTERIOR_WALL,
                TerritoryIntendedUse::HousingIndoor,
                false,
            ),
            Some(3)
        );
        assert_eq!(
            default_housing_appearance_target_slot(
                ITEM_UI_CATEGORY_FLOORING,
                TerritoryIntendedUse::HousingIndoor,
                false,
            ),
            Some(4)
        );
        assert_eq!(
            default_housing_appearance_target_slot(
                ITEM_UI_CATEGORY_CEILING_LIGHT,
                TerritoryIntendedUse::HousingIndoor,
                false,
            ),
            Some(5)
        );
    }

    #[test]
    fn housing_marker_target_slot_overrides_house_interior_fallback() {
        assert_eq!(
            housing_appearance_marker_target_slot(
                ITEM_UI_CATEGORY_CEILING_LIGHT,
                TerritoryIntendedUse::HousingIndoor,
                false,
                Some(2),
            ),
            Some(2)
        );
    }

    #[test]
    fn housing_marker_target_slot_rejects_category_mismatch() {
        assert_eq!(
            housing_appearance_marker_target_slot(
                ITEM_UI_CATEGORY_CEILING_LIGHT,
                TerritoryIntendedUse::HousingIndoor,
                false,
                Some(1),
            ),
            None
        );
    }

    #[test]
    fn housing_marker_target_slot_rejects_apartment_upper_floor_slot() {
        assert_eq!(
            housing_appearance_marker_target_slot(
                ITEM_UI_CATEGORY_CEILING_LIGHT,
                TerritoryIntendedUse::HousingIndoor,
                true,
                Some(5),
            ),
            None
        );
    }

    #[test]
    fn default_apartment_interior_appearance_target_uses_only_floor_slots() {
        assert_eq!(
            default_housing_appearance_target_slot(
                ITEM_UI_CATEGORY_INTERIOR_WALL,
                TerritoryIntendedUse::HousingIndoor,
                true,
            ),
            Some(0)
        );
        assert_eq!(
            default_housing_appearance_target_slot(
                ITEM_UI_CATEGORY_FLOORING,
                TerritoryIntendedUse::HousingIndoor,
                true,
            ),
            Some(1)
        );
        assert_eq!(
            default_housing_appearance_target_slot(
                ITEM_UI_CATEGORY_CEILING_LIGHT,
                TerritoryIntendedUse::HousingIndoor,
                true,
            ),
            Some(2)
        );
    }

    #[test]
    fn housing_exterior_appearance_slots_map_to_fields_categories_and_colors() {
        let expected = [
            (
                1,
                HousingExteriorField::Roof,
                ITEM_UI_CATEGORY_ROOF,
                HousingExteriorColorField::Roof,
            ),
            (
                2,
                HousingExteriorField::Walls,
                ITEM_UI_CATEGORY_EXTERIOR_WALL,
                HousingExteriorColorField::Walls,
            ),
            (
                3,
                HousingExteriorField::Windows,
                ITEM_UI_CATEGORY_WINDOW,
                HousingExteriorColorField::Windows,
            ),
            (
                4,
                HousingExteriorField::Door,
                ITEM_UI_CATEGORY_DOOR,
                HousingExteriorColorField::Door,
            ),
            (
                5,
                HousingExteriorField::RoofFixture,
                ITEM_UI_CATEGORY_ROOF_DECORATION,
                HousingExteriorColorField::RoofFixture,
            ),
            (
                6,
                HousingExteriorField::WallFixture,
                ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION,
                HousingExteriorColorField::WallFixture,
            ),
            (
                7,
                HousingExteriorField::AboveDoorBanner,
                ITEM_UI_CATEGORY_PLACARD,
                HousingExteriorColorField::AboveDoorBanner,
            ),
            (
                8,
                HousingExteriorField::Fence,
                ITEM_UI_CATEGORY_FENCE,
                HousingExteriorColorField::Fence,
            ),
        ];

        for (slot, field, category, color_field) in expected {
            assert_eq!(
                housing_exterior_field_for_appearance_slot(slot),
                Some(field)
            );
            assert_eq!(
                housing_exterior_item_ui_category_for_slot(slot),
                Some(category)
            );
            assert_eq!(
                housing_exterior_color_field_for_appearance_slot(slot),
                Some(color_field)
            );
        }

        assert_eq!(housing_exterior_field_for_appearance_slot(0), None);
    }

    #[test]
    fn housing_reset_cache_helper_clears_inventory_and_active_selection_when_requested() {
        let mut active_estate = Some(active_housing_estate(
            &HousingEstate {
                land_ident: 1005,
                house_id: house_id(5, 0, false).to_u64() as i64,
                ..Default::default()
            },
            false,
        ));
        let mut inventory = HousingInventory::default();
        *inventory
            .get_item_mut(ContainerType::HousingInteriorPlacedItems1, 0)
            .unwrap() = Item {
            quantity: 1,
            item_id: 1000,
            ..Default::default()
        };

        clear_housing_reset_cache(&mut active_estate, &mut inventory, true);

        assert!(active_estate.is_none());
        assert!(
            inventory
                .get_item(ContainerType::HousingInteriorPlacedItems1, 0)
                .unwrap()
                .is_empty_slot()
        );
    }

    #[test]
    fn placed_container_for_flat_slot_maps_indoor_pages() {
        assert_eq!(
            placed_container_for_flat_slot(50, true),
            Some((ContainerType::HousingInteriorPlacedItems2, 0))
        );
    }

    #[test]
    fn placed_container_for_flat_slot_maps_outdoor_slots() {
        assert_eq!(
            placed_container_for_flat_slot(0, false),
            Some((ContainerType::HousingExteriorPlacedItems, 0))
        );
        assert_eq!(
            placed_container_for_flat_slot(49, false),
            Some((ContainerType::HousingExteriorPlacedItems, 49))
        );
    }

    #[test]
    fn placed_container_for_flat_slot_rejects_out_of_range_slots() {
        assert_eq!(placed_container_for_flat_slot(600, true), None);
        assert_eq!(placed_container_for_flat_slot(50, false), None);
    }

    #[test]
    fn housing_indoor_entry_transform_keeps_house_and_apartment_defaults_separate() {
        let house = housing_indoor_entry_transform(false);
        let apartment = housing_indoor_entry_transform(true);

        assert_eq!(house.position, Position(Vec3::new(-0.25, -0.39, 5.0)));
        assert_eq!(house.rotation, 180.0);
        assert_eq!(apartment.position, Position(Vec3::ZERO));
        assert_eq!(apartment.rotation, 0.0);
    }

    #[test]
    fn interior_pattern_requests_are_allowed_from_house_yard() {
        assert!(housing_interior_pattern_area_allows_request(
            TerritoryIntendedUse::HousingOutdoor
        ));
        assert!(housing_interior_pattern_area_allows_request(
            TerritoryIntendedUse::HousingIndoor
        ));
        assert!(!housing_interior_pattern_area_allows_request(
            TerritoryIntendedUse::Inn
        ));
    }

    #[test]
    fn interior_pattern_apply_only_reloads_when_started_inside() {
        assert!(!housing_interior_pattern_apply_should_reload(
            TerritoryIntendedUse::HousingOutdoor
        ));
        assert!(housing_interior_pattern_apply_should_reload(
            TerritoryIntendedUse::HousingIndoor
        ));
    }

    #[test]
    fn outdoor_interior_pattern_can_use_owned_house_without_active_ward_context() {
        let mut database = WorldDatabase::new_at(":memory:");
        let mut owned_estate = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        owned_estate.owner_content_id = Some(100);
        owned_estate.land_ident = 55;
        database.insert_housing_estate_for_test(owned_estate.clone());

        let resolved =
            active_housing_estate_for_owned_outdoor_pattern(&mut database, None, 100).unwrap();

        assert_eq!(resolved.land_ident, owned_estate.land_ident);
        assert_eq!(
            resolved.house_id,
            HouseId::from_u64(owned_estate.house_id as u64)
        );
        assert!(!resolved.indoors);
    }

    #[test]
    fn housing_indoor_init_resolves_when_active_estate_is_missing_or_outdoor() {
        let estate = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        let outdoor = active_housing_estate(&estate, false);
        let indoor = active_housing_estate(&estate, true);

        assert!(housing_indoor_init_needs_resolution(None));
        assert!(housing_indoor_init_needs_resolution(Some(&outdoor)));
        assert!(!housing_indoor_init_needs_resolution(Some(&indoor)));
    }

    #[test]
    fn minimal_housing_interior_details_use_plain_white_large_preset() {
        let details = housing_interior_details(3);

        assert_eq!(details.light_level, 3);
        assert_eq!(details.ground_walls, 66111);
        assert_eq!(details.ground_floor, 65591);
        assert_eq!(details.ground_chandelier, 65848);
        assert_eq!(details.top_walls, 66111);
        assert_eq!(details.top_floor, 65591);
        assert_eq!(details.top_chandelier, 65848);
        assert_eq!(details.cellar_walls, 66111);
        assert_eq!(details.cellar_floor, 65591);
        assert_eq!(details.cellar_chandelier, 65848);
        assert_eq!(details.unk_interior, 65848);
    }

    #[test]
    fn housing_interior_details_uses_interior_json_over_minimal_defaults() {
        let json = r#"{
            "window_style": 1,
            "door_style": 2,
            "door_stain": 3,
            "ground_walls": 10,
            "ground_floor": 11,
            "ground_chandelier": 12,
            "top_walls": 20,
            "top_floor": 21,
            "top_chandelier": 22,
            "cellar_walls": 30,
            "cellar_floor": 31,
            "cellar_chandelier": 32
        }"#;

        let details = housing_interior_details_from_json(json, 4, false);

        assert_eq!(details.window_style, 1);
        assert_eq!(details.door_style, 2);
        assert_eq!(details.door_stain, 3);
        assert_eq!(details.light_level, 4);
        assert_eq!(details.ground_walls, 10);
        assert_eq!(details.ground_floor, 11);
        assert_eq!(details.ground_chandelier, 12);
        assert_eq!(details.top_walls, 20);
        assert_eq!(details.top_floor, 21);
        assert_eq!(details.top_chandelier, 22);
        assert_eq!(details.cellar_walls, 30);
        assert_eq!(details.cellar_floor, 31);
        assert_eq!(details.cellar_chandelier, 32);
    }

    #[test]
    fn invalid_housing_interior_json_falls_back_to_minimal_defaults() {
        let details = housing_interior_details_from_json("{", 2, false);

        assert_eq!(details.light_level, 2);
        assert_eq!(details.ground_walls, MINIMAL_INTERIOR_WALL);
        assert_eq!(details.ground_floor, MINIMAL_INTERIOR_FLOOR);
        assert_eq!(details.ground_chandelier, MINIMAL_INTERIOR_LIGHT);
        assert_eq!(details.top_walls, MINIMAL_INTERIOR_WALL);
        assert_eq!(details.top_floor, MINIMAL_INTERIOR_FLOOR);
        assert_eq!(details.top_chandelier, MINIMAL_INTERIOR_LIGHT);
        assert_eq!(details.cellar_walls, MINIMAL_INTERIOR_WALL);
        assert_eq!(details.cellar_floor, MINIMAL_INTERIOR_FLOOR);
        assert_eq!(details.cellar_chandelier, MINIMAL_INTERIOR_LIGHT);
        assert_eq!(details.unk_interior, MINIMAL_INTERIOR_LIGHT);
    }

    #[test]
    fn apartment_interior_json_does_not_populate_top_or_cellar_floors() {
        let json = r#"{
            "window_style": 7,
            "door_style": 8,
            "door_stain": 9,
            "ground_walls": 100,
            "ground_floor": 101,
            "ground_chandelier": 102,
            "top_walls": 200,
            "top_floor": 201,
            "top_chandelier": 202,
            "cellar_walls": 300,
            "cellar_floor": 301,
            "cellar_chandelier": 302
        }"#;

        let details = housing_interior_details_from_json(json, 1, true);

        assert_eq!(details.window_style, 7);
        assert_eq!(details.door_style, 8);
        assert_eq!(details.door_stain, 9);
        assert_eq!(details.light_level, 1);
        assert_eq!(details.ground_walls, 100);
        assert_eq!(details.ground_floor, 101);
        assert_eq!(details.ground_chandelier, 102);
        assert_eq!(details.top_walls, 0);
        assert_eq!(details.top_floor, 0);
        assert_eq!(details.top_chandelier, 0);
        assert_eq!(details.cellar_walls, 0);
        assert_eq!(details.cellar_floor, 0);
        assert_eq!(details.cellar_chandelier, 0);
        assert_eq!(details.unk_interior, 0);
    }

    #[test]
    fn update_exterior_json_color_preserves_existing_fields() {
        let updated = update_exterior_json_color(
            r#"{"roof_id":12,"colors":{"roof":3,"walls":4}}"#,
            HousingExteriorColorField::Walls,
            9,
        )
        .unwrap();

        let exterior: HouseExteriorJson = serde_json::from_str(&updated).unwrap();

        assert_eq!(exterior.roof_id, Some(12));
        assert_eq!(
            exterior.colors.as_ref().and_then(|colors| colors.roof),
            Some(3)
        );
        assert_eq!(exterior.colors.and_then(|colors| colors.walls), Some(9));
    }

    #[test]
    fn update_interior_json_field_preserves_existing_fields() {
        let updated = update_interior_json_field(
            r#"{"window_style":1,"ground_floor":65591}"#,
            HousingInteriorField::GroundWalls,
            66111,
        )
        .unwrap();

        let interior: HouseInteriorJson = serde_json::from_str(&updated).unwrap();

        assert_eq!(interior.window_style, Some(1));
        assert_eq!(interior.ground_floor, Some(65591));
        assert_eq!(interior.ground_walls, Some(66111));
    }

    #[test]
    fn update_interior_json_renovation_row_id_preserves_fixture_fields() {
        let updated = update_interior_json_renovation_row_id(
            r#"{"window_style":1,"ground_floor":65591,"ground_walls":66111}"#,
            18,
        )
        .unwrap();

        let interior: HouseInteriorJson = serde_json::from_str(&updated).unwrap();

        assert_eq!(interior.renovation_row_id, Some(18));
        assert_eq!(interior.window_style, Some(1));
        assert_eq!(interior.ground_floor, Some(65591));
        assert_eq!(interior.ground_walls, Some(66111));
        assert_eq!(
            housing_interior_renovation_row_id_from_json(&updated),
            Some(18)
        );
    }

    #[test]
    fn invalid_exterior_json_mutation_returns_error_and_keeps_existing_string() {
        let existing_json = "{";
        let outcome = update_exterior_json_field(existing_json, HousingExteriorField::Roof, 9);

        assert!(
            outcome.is_error(),
            "malformed exterior json must not be treated as a writable default"
        );

        let persisted_json = outcome
            .into_success()
            .unwrap_or_else(|| existing_json.to_string());
        assert_eq!(persisted_json, existing_json);
    }

    #[test]
    fn invalid_interior_json_mutation_returns_error_and_keeps_existing_string() {
        let existing_json = "{";
        let outcome =
            update_interior_json_field(existing_json, HousingInteriorField::GroundWalls, 66111);

        assert!(
            outcome.is_error(),
            "malformed interior json must not be treated as a writable default"
        );

        let persisted_json = outcome
            .into_success()
            .unwrap_or_else(|| existing_json.to_string());
        assert_eq!(persisted_json, existing_json);
    }

    #[test]
    fn additional_chambers_door_is_hidden_for_personal_estates_only() {
        assert!(should_hide_additional_chambers_door(0x0B));
        assert!(!should_hide_additional_chambers_door(0x1B));
    }

    #[test]
    fn housing_edit_permission_requires_matching_owner() {
        let mut estate = estate(house_id(5, 0, false), 0x0B, false);
        estate.owner_content_id = Some(100);

        assert!(can_edit_housing_estate(&estate, 100));
        assert!(!can_edit_housing_estate(&estate, 200));

        estate.owner_content_id = None;
        assert!(!can_edit_housing_estate(&estate, 100));
    }

    #[test]
    fn outdoor_housing_location_from_raw_entry_normalizes_land_set_entries() {
        let context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: 2,
            division: 0,
        };

        assert_eq!(
            outdoor_housing_location_from_raw_entry(context, 5),
            Some(OutdoorHousingLocation {
                territory_type_id: 340,
                ward_index: 2,
                division: 0,
                plot_index: 5,
                raw_plot_index: 5,
            })
        );
        assert_eq!(
            outdoor_housing_location_from_raw_entry(context, 35),
            Some(OutdoorHousingLocation {
                territory_type_id: 340,
                ward_index: 2,
                division: 1,
                plot_index: 5,
                raw_plot_index: 35,
            })
        );
        assert_eq!(outdoor_housing_location_from_raw_entry(context, 60), None);
        assert_eq!(outdoor_housing_location_from_raw_entry(context, 0xFF), None);
    }

    #[test]
    fn outdoor_edit_row_requires_resolved_location_and_owner() {
        let main_location = OutdoorHousingLocation {
            territory_type_id: 340,
            ward_index: 2,
            division: 0,
            plot_index: 5,
            raw_plot_index: 5,
        };
        let subdivision_location = OutdoorHousingLocation {
            division: 1,
            raw_plot_index: 35,
            ..main_location
        };
        let mut main_estate = ward_estate(5, 0, 0x0B);
        main_estate.owner_content_id = Some(100);
        let mut subdivision_estate = ward_estate(5, 1, 0x0B);
        subdivision_estate.owner_content_id = Some(100);

        let active =
            active_housing_estate_for_outdoor_row(&main_estate, main_location, 100).unwrap();
        assert_eq!(active.land_ident, main_estate.land_ident);
        assert_eq!(
            active.house_id,
            HouseId::from_u64(main_estate.house_id as u64)
        );
        assert!(!active.indoors);

        assert!(
            active_housing_estate_for_outdoor_row(&main_estate, main_location, 200).is_none(),
            "wrong owner must not edit the resolved outdoor estate"
        );
        assert!(
            active_housing_estate_for_outdoor_row(&main_estate, subdivision_location, 100)
                .is_none(),
            "main-division estates must not satisfy subdivision packet entries"
        );
        assert!(
            active_housing_estate_for_outdoor_row(&subdivision_estate, subdivision_location, 100)
                .is_some()
        );
    }

    #[test]
    fn outdoor_edit_row_normalizes_subdivision_house_id_to_raw_landset_entry() {
        let subdivision_location = OutdoorHousingLocation {
            territory_type_id: 340,
            ward_index: 2,
            division: 1,
            plot_index: 5,
            raw_plot_index: 35,
        };
        let mut subdivision_estate = ward_estate(5, 1, 0x0B);
        subdivision_estate.owner_content_id = Some(100);
        subdivision_estate.house_id = house_id(5, 0, false).to_u64() as i64;

        let active =
            active_housing_estate_for_outdoor_row(&subdivision_estate, subdivision_location, 100)
                .unwrap();

        assert_eq!(active.house_id, house_id(35, 0, false));
        assert!(!active.indoors);
    }

    #[test]
    fn outdoor_edit_resolution_requires_active_ward_context_even_with_owned_default_estate() {
        let mut estate = ward_estate(
            DEFAULT_LOCAL_HOUSING_PLOT_INDEX as i32,
            DEFAULT_LOCAL_HOUSING_DIVISION as i32,
            DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
        );
        estate.ward_index = DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32;
        estate.owner_content_id = Some(100);

        let mut database = WorldDatabase::new_at(":memory:");
        database.insert_housing_estate_for_test(estate);

        assert!(
            resolve_active_housing_estate_for_outdoor_edit(
                &mut database,
                None,
                21,
                DEFAULT_LOCAL_HOUSING_PLOT_INDEX,
                100,
            )
            .is_none(),
            "outdoor placement/edit must not fabricate default local ward context for authoritative writes"
        );
    }

    #[test]
    fn ward_browse_context_is_display_only_and_does_not_authorize_outdoor_writes() {
        let mut estate = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        estate.owner_content_id = Some(100);

        let mut database = WorldDatabase::new_at(":memory:");
        database.insert_housing_estate_for_test(estate);

        let browse_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: 2,
            division: 0,
        };
        let default_context = ActiveHousingWardContext {
            territory_type_id: 999,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        };

        assert_eq!(
            display_housing_ward_context_or_default(
                Some(browse_context),
                None,
                None,
                default_context
            ),
            browse_context,
            "Lua/display lookups should keep the browsed ward context"
        );
        assert!(
            trusted_housing_ward_context_after_display_update(None, browse_context).is_none(),
            "ward browsing must not create outdoor write authority"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_owner_gate(
                &mut database,
                trusted_housing_ward_context_after_display_update(None, browse_context),
                21,
                &ActiveHousingEstate {
                    land_ident: house_id(5, 0, false).to_u64() as i64,
                    house_id: house_id(5, 0, false),
                    indoors: false,
                },
                100,
            )
            .is_none(),
            "owner gate must ignore display-only ward context"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_edit(
                &mut database,
                trusted_housing_ward_context_after_display_update(None, browse_context),
                21,
                5,
                100,
            )
            .is_none(),
            "outdoor placement/edit must ignore display-only ward context"
        );
    }

    #[test]
    fn vacant_placard_context_is_display_only_and_does_not_authorize_outdoor_writes() {
        let mut estate = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        estate.owner_content_id = Some(100);

        let mut database = WorldDatabase::new_at(":memory:");
        database.insert_housing_estate_for_test(estate);

        let vacant_placard_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: 2,
            division: 0,
        };

        assert!(
            trusted_housing_ward_context_after_vacant_placard(None, vacant_placard_context)
                .is_none(),
            "vacant placard display must not create outdoor write authority"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_edit(
                &mut database,
                trusted_housing_ward_context_after_vacant_placard(None, vacant_placard_context),
                21,
                5,
                100,
            )
            .is_none(),
            "outdoor placement/edit must ignore vacant placard display context"
        );
    }

    #[test]
    fn non_owner_occupied_placard_selection_is_display_only_and_preserves_exit_target() {
        let mut owned_estate = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        owned_estate.owner_content_id = Some(100);
        owned_estate.land_ident = 55;

        let mut browsed_estate = ward_estate(6, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        browsed_estate.owner_content_id = Some(200);
        browsed_estate.land_ident = 66;

        let existing_active_estate = active_housing_estate(&owned_estate, true);

        assert!(
            placard_authoritative_estate(&browsed_estate, 100).is_none(),
            "non-owner occupied placards must remain display-only"
        );
        assert_eq!(
            existing_active_estate.house_id,
            HouseId::from_u64(owned_estate.house_id as u64),
            "exit resolution should keep targeting the pre-existing authoritative estate"
        );
    }

    #[test]
    fn owner_occupied_placard_selection_promotes_authoritative_context() {
        let mut estate = ward_estate(6, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        estate.owner_content_id = Some(100);
        estate.land_ident = 66;

        let active = placard_authoritative_estate(&estate, 100)
            .expect("owner occupied placard should establish authority");

        assert_eq!(active.land_ident, estate.land_ident);
        assert_eq!(
            active.house_id,
            outdoor_house_id_from_estate(&estate),
            "placard authority should use the outdoor placard house id"
        );
        assert!(!active.indoors);
    }

    #[test]
    fn outdoor_init_does_not_promote_default_context_to_authoritative_edit_context() {
        let mut estate = ward_estate(
            DEFAULT_LOCAL_HOUSING_PLOT_INDEX as i32,
            DEFAULT_LOCAL_HOUSING_DIVISION as i32,
            DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
        );
        estate.territory_type_id = 340;
        estate.world_id = 21;
        estate.ward_index = DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32;
        estate.owner_content_id = Some(100);

        let mut database = WorldDatabase::new_at(":memory:");
        database.insert_housing_estate_for_test(estate.clone());

        let display_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        };
        let active_estate = active_housing_estate(&estate, false);
        let authoritative_context = outdoor_init_authoritative_context(None, display_context, 340);

        assert!(
            authoritative_context.is_none(),
            "outdoor init must not turn a default local context into edit authority"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_owner_gate(
                &mut database,
                authoritative_context,
                21,
                &active_estate,
                100,
            )
            .is_none(),
            "owner gate must reject bootstrap estate without a real active ward context"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_edit(
                &mut database,
                authoritative_context,
                21,
                DEFAULT_LOCAL_HOUSING_PLOT_INDEX,
                100,
            )
            .is_none(),
            "outdoor edit must reject bootstrap estate without a real active ward context"
        );

        let explicit_context =
            outdoor_init_authoritative_context(Some(display_context), display_context, 340);

        assert!(
            resolve_active_housing_estate_for_outdoor_owner_gate(
                &mut database,
                explicit_context,
                21,
                &active_estate,
                100,
            )
            .is_some(),
            "a trusted explicit outdoor context should still authorize the owner gate"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_edit(
                &mut database,
                explicit_context,
                21,
                DEFAULT_LOCAL_HOUSING_PLOT_INDEX,
                100,
            )
            .is_some(),
            "a trusted explicit outdoor context should still authorize plot edits"
        );
    }

    #[test]
    fn outdoor_init_display_prefers_matching_active_context_over_stale_display_context() {
        let active_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: 2,
            division: 0,
        };
        let stale_display_context = ActiveHousingWardContext {
            territory_type_id: 341,
            ward_index: 9,
            division: 1,
        };
        let default_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        };

        assert_eq!(
            outdoor_init_display_context(
                Some(active_context),
                Some(stale_display_context),
                default_context,
                340,
            ),
            active_context,
            "outdoor init HouseList selection should trust active context for the entered zone"
        );
    }

    #[test]
    fn outdoor_init_display_can_use_display_context_without_active_context() {
        let display_context = ActiveHousingWardContext {
            territory_type_id: 341,
            ward_index: 9,
            division: 1,
        };
        let default_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        };

        assert_eq!(
            outdoor_init_display_context(None, Some(display_context), default_context, 340),
            ActiveHousingWardContext {
                territory_type_id: 340,
                ..display_context
            },
            "display context should still select the HouseList ward when no trusted active context exists"
        );
    }

    #[test]
    fn outdoor_init_display_ignores_bootstrap_active_estate_context_without_authority() {
        let mut estate = ward_estate(
            DEFAULT_LOCAL_HOUSING_PLOT_INDEX as i32,
            DEFAULT_LOCAL_HOUSING_DIVISION as i32,
            DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
        );
        estate.territory_type_id = 340;
        estate.world_id = 21;
        estate.ward_index = DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32;
        estate.owner_content_id = Some(100);

        let mut database = WorldDatabase::new_at(":memory:");
        database.insert_housing_estate_for_test(estate.clone());

        let bootstrap_context = active_housing_ward_context_from_estate(&estate);
        let display_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: 9,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        };
        let default_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        };

        assert_eq!(
            outdoor_init_display_context(
                Some(bootstrap_context),
                Some(display_context),
                default_context,
                340,
            ),
            bootstrap_context,
            "the old call shape would trust the bootstrap local estate context"
        );

        let active_context = outdoor_init_active_context(None, Some(bootstrap_context));
        let selected_context = outdoor_init_display_context(
            active_context,
            Some(display_context),
            default_context,
            340,
        );

        assert_eq!(
            selected_context, display_context,
            "outdoor init HouseList selection must use display context when there is no trusted active context"
        );

        let authoritative_context = outdoor_init_authoritative_context(None, selected_context, 340);
        let active_estate = active_housing_estate(&estate, false);

        assert!(
            authoritative_context.is_none(),
            "outdoor init display selection must not create outdoor write authority"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_owner_gate(
                &mut database,
                authoritative_context,
                21,
                &active_estate,
                100,
            )
            .is_none(),
            "owner gate must reject bootstrap estate without a real active ward context"
        );
        assert!(
            resolve_active_housing_estate_for_outdoor_edit(
                &mut database,
                authoritative_context,
                21,
                DEFAULT_LOCAL_HOUSING_PLOT_INDEX,
                100,
            )
            .is_none(),
            "outdoor edit must reject bootstrap estate without a real active ward context"
        );
    }

    #[test]
    fn outdoor_init_clears_stale_active_context_when_entering_different_housing_zone() {
        let stale_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: 2,
            division: 0,
        };
        let display_context = ActiveHousingWardContext {
            territory_type_id: 341,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
        };

        assert!(
            outdoor_init_authoritative_context(Some(stale_context), display_context, 341).is_none(),
            "stale active housing context from another outdoor zone must not survive transition"
        );
        assert_eq!(
            display_housing_ward_context_or_default(
                Some(display_context),
                None,
                None,
                stale_context
            ),
            display_context,
            "display context can still select the HouseList ward after stale authority is cleared"
        );
    }

    #[test]
    fn outdoor_edit_rejects_stale_context() {
        let mut estate = ward_estate(5, 1, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        estate.ward_index = DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32;
        estate.owner_content_id = Some(100);

        let mut database = WorldDatabase::new_at(":memory:");
        database.insert_housing_estate_for_test(estate);

        assert!(
            resolve_active_housing_estate_for_outdoor_edit(&mut database, None, 21, 35, 100)
                .is_none(),
            "outdoor translation/edit must reject stale ward context for raw subdivision plot ids"
        );
    }

    #[test]
    fn outdoor_owner_gate_allows_subdivision_raw_house_id_with_stale_db_house_id() {
        let stale_house_id = house_id(5, 0, false);
        let raw_subdivision_house_id = house_id(35, 0, false);
        let mut subdivision_estate = ward_estate(5, 1, 0x0B);
        subdivision_estate.owner_content_id = Some(100);
        subdivision_estate.house_id = stale_house_id.to_u64() as i64;
        subdivision_estate.land_ident = stale_house_id.to_u64() as i64;

        let mut database = WorldDatabase::new_at(":memory:");
        database.insert_housing_estate_for_test(subdivision_estate.clone());
        assert!(
            database
                .housing_estate_by_house_id(raw_subdivision_house_id)
                .is_none(),
            "regression setup must fail if the gate looks up the DB row by raw active HouseId"
        );
        assert!(
            database
                .housing_estate_by_location(340, 21, 2, 1, 5)
                .is_some(),
            "regression setup must be resolvable by outdoor location"
        );

        let active_estate = ActiveHousingEstate {
            land_ident: subdivision_estate.land_ident,
            house_id: raw_subdivision_house_id,
            indoors: false,
        };
        let active_ward_context = ActiveHousingWardContext {
            territory_type_id: 340,
            ward_index: 2,
            division: 1,
        };
        let (resolved, location) = resolve_active_housing_estate_for_outdoor_owner_gate(
            &mut database,
            Some(active_ward_context),
            21,
            &active_estate,
            100,
        )
        .expect(
            "subdivision outdoor owner gate should resolve by location, not raw active HouseId",
        );

        assert_eq!(resolved.land_ident, subdivision_estate.land_ident);
        assert_eq!(resolved.house_id, raw_subdivision_house_id);
        assert!(!resolved.indoors);
        assert_eq!(location.division, 1);
        assert_eq!(location.plot_index, 5);
        assert_eq!(location.raw_plot_index, 35);
    }

    #[test]
    fn build_outdoor_estate_furniture_lists_uses_estate_house_id_and_exterior_placed_rows() {
        let estate = ward_estate(5, 0, 0x0B);
        let position = Position(Vec3::new(1.0, 2.0, 3.0));
        let rows = vec![
            HousingFurniture {
                container_type: ContainerType::HousingExteriorPlacedItems as i32,
                catalog_id: 321,
                pos_x: position.0.x,
                pos_y: position.0.y,
                pos_z: position.0.z,
                rotation: 0.5,
                placed: true,
                ..Default::default()
            },
            HousingFurniture {
                container_type: ContainerType::HousingInteriorPlacedItems1 as i32,
                catalog_id: 654,
                placed: true,
                ..Default::default()
            },
            HousingFurniture {
                container_type: ContainerType::HousingExteriorPlacedItems as i32,
                catalog_id: 999,
                placed: false,
                ..Default::default()
            },
        ];

        let lists = build_outdoor_estate_furniture_lists(&estate, &rows);

        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, HouseId::from_u64(estate.house_id as u64));
        assert_eq!(lists[0].unk2, 0);
        assert_eq!(lists[0].furniture.len(), 1);
        assert_eq!(lists[0].furniture[0].id, 321);
        assert_eq!(lists[0].furniture[0].position, position);
        assert_eq!(lists[0].furniture[0].rotation, 0.5);
    }

    #[test]
    fn outdoor_furniture_lists_normalize_subdivision_house_id() {
        let mut estate = ward_estate(5, 1, 0x0B);
        estate.house_id = house_id(5, 0, false).to_u64() as i64;
        let rows = vec![HousingFurniture {
            container_type: ContainerType::HousingExteriorPlacedItems as i32,
            catalog_id: 321,
            placed: true,
            ..Default::default()
        }];

        let lists = build_outdoor_estate_furniture_lists(&estate, &rows);

        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, house_id(35, 0, false));
    }

    #[test]
    fn resolve_active_indoor_housing_estate_preserves_active_apartment_over_owned_house() {
        let mut database = WorldDatabase::new_at(":memory:");
        let apartment = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(0, 9, true).to_u64() as i64,
            house_id: house_id(0, 9, true).to_u64() as i64,
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 0,
            room_number: 9,
            is_apartment: true,
            owner_content_id: Some(100),
            owner_name: "Apartment Owner".to_string(),
            estate_name: "Apartment".to_string(),
            ..Default::default()
        });
        let _house = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(5, 0, false).to_u64() as i64,
            house_id: house_id(5, 0, false).to_u64() as i64,
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 5,
            room_number: 0,
            is_apartment: false,
            owner_content_id: Some(100),
            owner_name: "House Owner".to_string(),
            estate_name: "House".to_string(),
            ..Default::default()
        });
        let active_apartment = ActiveHousingEstate {
            land_ident: apartment.land_ident,
            house_id: HouseId::from_u64(apartment.house_id as u64),
            indoors: true,
        };

        let resolved = resolve_active_indoor_housing_estate(
            &mut database,
            Some(&active_apartment),
            None,
            609,
            21,
            100,
        )
        .expect("active apartment should resolve");

        assert_eq!(resolved.land_ident, apartment.land_ident);
        assert!(resolved.house_id.unit.apartment_flag);
        assert_eq!(resolved.house_id.room_number, 9);
        assert!(resolved.indoors);
    }

    #[test]
    fn resolve_active_indoor_housing_estate_prefers_owned_apartment_for_apartment_interiors_without_active_estate()
     {
        let mut database = WorldDatabase::new_at(":memory:");
        let apartment = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(0, 4, true).to_u64() as i64,
            house_id: house_id(0, 4, true).to_u64() as i64,
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 0,
            room_number: 4,
            is_apartment: true,
            owner_content_id: Some(100),
            owner_name: "Apartment Owner".to_string(),
            estate_name: "Apartment".to_string(),
            ..Default::default()
        });
        let house = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(5, 0, false).to_u64() as i64,
            house_id: house_id(5, 0, false).to_u64() as i64,
            territory_type_id: 341,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 5,
            room_number: 0,
            is_apartment: false,
            owner_content_id: Some(100),
            owner_name: "House Owner".to_string(),
            estate_name: "House".to_string(),
            ..Default::default()
        });

        let resolved =
            resolve_active_indoor_housing_estate(&mut database, None, None, 609, 21, 100)
                .expect("apartment interior fallback should resolve an owned apartment");

        assert_eq!(resolved.land_ident, apartment.land_ident);
        assert_ne!(resolved.land_ident, house.land_ident);
        assert!(resolved.house_id.unit.apartment_flag);
        assert_eq!(resolved.house_id.room_number, 4);
        assert!(resolved.indoors);
    }

    #[test]
    fn resolve_indoor_estate_prefers_matching_subdivision_apartment() {
        let mut database = WorldDatabase::new_at(":memory:");
        let main_division_apartment = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(0, 1, true).to_u64() as i64,
            house_id: house_id(0, 1, true).to_u64() as i64,
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 0,
            room_number: 1,
            is_apartment: true,
            owner_content_id: Some(100),
            owner_name: "Apartment Owner".to_string(),
            estate_name: "Main Division Apartment".to_string(),
            ..Default::default()
        });
        let subdivision_apartment = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(HOUSING_PLOTS_PER_DIVISION, 1, true).to_u64() as i64,
            house_id: house_id(HOUSING_PLOTS_PER_DIVISION, 1, true).to_u64() as i64,
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 1,
            plot_index: HOUSING_PLOTS_PER_DIVISION as i32,
            room_number: 1,
            is_apartment: true,
            owner_content_id: Some(100),
            owner_name: "Apartment Owner".to_string(),
            estate_name: "Subdivision Apartment".to_string(),
            ..Default::default()
        });

        let resolved = resolve_active_indoor_housing_estate(
            &mut database,
            None,
            Some(ActiveHousingWardContext {
                territory_type_id: 340,
                ward_index: 2,
                division: 1,
            }),
            609,
            21,
            100,
        )
        .expect("subdivision apartment interior fallback should resolve the subdivision apartment");

        assert_eq!(resolved.land_ident, subdivision_apartment.land_ident);
        assert_ne!(resolved.land_ident, main_division_apartment.land_ident);
        assert!(resolved.house_id.unit.apartment_flag);
        assert_eq!(resolved.house_id.room_number, 1);
        assert!(resolved.indoors);
    }

    #[test]
    fn resolve_active_indoor_housing_estate_keeps_house_first_for_non_apartment_interiors_without_active_estate()
     {
        let mut database = WorldDatabase::new_at(":memory:");
        let apartment = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(0, 4, true).to_u64() as i64,
            house_id: house_id(0, 4, true).to_u64() as i64,
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 0,
            room_number: 4,
            is_apartment: true,
            owner_content_id: Some(100),
            owner_name: "Apartment Owner".to_string(),
            estate_name: "Apartment".to_string(),
            ..Default::default()
        });
        let house = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_id(5, 0, false).to_u64() as i64,
            house_id: house_id(5, 0, false).to_u64() as i64,
            territory_type_id: 341,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 5,
            room_number: 0,
            is_apartment: false,
            owner_content_id: Some(100),
            owner_name: "House Owner".to_string(),
            estate_name: "House".to_string(),
            ..Default::default()
        });

        let resolved = resolve_active_indoor_housing_estate(
            &mut database,
            None,
            None,
            DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_LARGE,
            21,
            100,
        )
        .expect("house interior fallback should resolve the owned house");

        assert_eq!(resolved.land_ident, house.land_ident);
        assert_ne!(resolved.land_ident, apartment.land_ident);
        assert!(!resolved.house_id.unit.apartment_flag);
        assert!(resolved.indoors);
    }

    #[test]
    fn outdoor_item_removal_requires_active_outdoor_owner_estate() {
        let mut estate = ward_estate(5, 0, 0x0B);
        estate.land_ident = 55;
        estate.owner_content_id = Some(100);
        let active = ActiveHousingEstate {
            land_ident: estate.land_ident,
            house_id: house_id(5, 0, false),
            indoors: false,
        };
        let indoor_active = ActiveHousingEstate {
            indoors: true,
            ..active.clone()
        };

        assert!(
            active_housing_estate_for_outdoor_removal_row(None, Some(&estate), 100).is_none(),
            "outdoor removal must not resolve a fixed plot without active outdoor estate context"
        );
        assert!(
            active_housing_estate_for_outdoor_removal_row(Some(&indoor_active), Some(&estate), 100)
                .is_none(),
            "indoor active estates must not authorize outdoor removals"
        );
        assert!(
            active_housing_estate_for_outdoor_removal_row(Some(&active), Some(&estate), 200)
                .is_none(),
            "non-owners must not remove outdoor furniture"
        );
        assert!(
            active_housing_estate_for_outdoor_removal_row(Some(&active), None, 100).is_none(),
            "missing DB estate row must reject outdoor removal"
        );

        let resolved =
            active_housing_estate_for_outdoor_removal_row(Some(&active), Some(&estate), 100)
                .unwrap();

        assert_eq!(resolved.land_ident, estate.land_ident);
        assert_eq!(resolved.house_id, house_id(5, 0, false));
        assert!(!resolved.indoors);
    }

    #[test]
    fn housing_item_removal_broadcast_is_only_needed_for_placed_containers() {
        assert!(should_broadcast_housing_item_removal(
            ContainerType::HousingExteriorPlacedItems
        ));
        assert!(should_broadcast_housing_item_removal(
            ContainerType::HousingInteriorPlacedItems1
        ));
        assert!(should_broadcast_housing_item_removal(
            ContainerType::HousingInteriorPlacedItems12
        ));
        assert!(!should_broadcast_housing_item_removal(
            ContainerType::HousingExteriorStoreroom
        ));
        assert!(!should_broadcast_housing_item_removal(
            ContainerType::HousingInteriorStoreroom1
        ));
    }

    #[test]
    fn selected_or_owned_housing_estate_prefers_active_parameterized_estate() {
        let mut database = WorldDatabase::new_at(":memory:");
        let default_estate = database.ensure_local_estate(100, "Tester", 67);
        let selected_estate = database.ensure_local_estate_with_spec(HousingEstateSpec {
            owner_content_id: 100,
            owner_name: "Tester FC".to_string(),
            world_id: 67,
            territory_type_id: 341,
            ward_index: 2,
            division: 0,
            plot_index: 12,
            plot_size: PlotSize::Medium,
            free_company: true,
        });
        let active_estate = active_housing_estate(&selected_estate, false);

        let resolved =
            selected_or_owned_housing_estate(&mut database, Some(&active_estate), 100).unwrap();

        assert_eq!(resolved.land_ident, selected_estate.land_ident);
        assert_ne!(resolved.land_ident, default_estate.land_ident);
        assert_eq!(active_estate.house_id.territory_type_id, 341);
        assert_eq!(active_estate.house_id.ward_index, 2);
        assert_eq!(
            active_estate.house_id.unit.apartment_division_plot_index,
            12
        );
        assert_eq!(resolved.plot_size, PlotSize::Medium as i32);
        assert_eq!(
            resolved.flags & FREE_COMPANY_HOUSING_FLAG,
            FREE_COMPANY_HOUSING_FLAG
        );
    }

    #[test]
    fn selected_or_owned_housing_estate_ignores_foreign_active_estate_and_returns_owned_estate() {
        let mut database = WorldDatabase::new_at(":memory:");
        let owned_estate = database.ensure_local_estate(100, "Tester", 67);
        let foreign_estate = database.ensure_local_estate_with_spec(HousingEstateSpec {
            owner_content_id: 200,
            owner_name: "Other Owner".to_string(),
            world_id: 67,
            territory_type_id: 341,
            ward_index: 2,
            division: 0,
            plot_index: 12,
            plot_size: PlotSize::Medium,
            free_company: false,
        });
        let active_estate = active_housing_estate(&foreign_estate, false);

        let resolved =
            selected_or_owned_housing_estate(&mut database, Some(&active_estate), 100).unwrap();

        assert_eq!(resolved.land_ident, owned_estate.land_ident);
        assert_ne!(resolved.land_ident, foreign_estate.land_ident);
        assert_eq!(resolved.owner_content_id, Some(100));
    }

    #[test]
    fn selected_or_owned_housing_estate_prefers_house_over_apartment_without_active_estate() {
        let mut database = WorldDatabase::new_at(":memory:");
        let apartment_house_id = HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: 4,
                apartment_flag: true,
            },
            unk1: 0,
            ward_index: 2,
            room_number: 12,
            territory_type_id: 340,
            world_id: 21,
        };
        let house_house_id = HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: 5,
                apartment_flag: false,
            },
            unk1: 0,
            ward_index: 2,
            room_number: 0,
            territory_type_id: 341,
            world_id: 21,
        };

        let apartment_estate = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: apartment_house_id.to_u64() as i64,
            house_id: apartment_house_id.to_u64() as i64,
            territory_type_id: apartment_house_id.territory_type_id as i32,
            world_id: apartment_house_id.world_id as i32,
            ward_index: apartment_house_id.ward_index as i32,
            division: 0,
            plot_index: 4,
            room_number: apartment_house_id.room_number as i32,
            is_apartment: true,
            owner_content_id: Some(100),
            owner_name: "Apartment Owner".to_string(),
            estate_name: "Apartment".to_string(),
            ..Default::default()
        });
        let house_estate = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: house_house_id.to_u64() as i64,
            house_id: house_house_id.to_u64() as i64,
            territory_type_id: house_house_id.territory_type_id as i32,
            world_id: house_house_id.world_id as i32,
            ward_index: house_house_id.ward_index as i32,
            division: 0,
            plot_index: 5,
            room_number: house_house_id.room_number as i32,
            is_apartment: false,
            owner_content_id: Some(100),
            owner_name: "House Owner".to_string(),
            estate_name: "House".to_string(),
            ..Default::default()
        });

        let resolved = selected_or_owned_housing_estate(&mut database, None, 100).unwrap();

        assert_eq!(resolved.land_ident, house_estate.land_ident);
        assert_ne!(resolved.land_ident, apartment_estate.land_ident);
        assert!(!resolved.is_apartment);
    }

    #[test]
    fn selected_or_owned_housing_estate_prefers_ordered_non_apartment_rows_before_apartments() {
        let mut database = WorldDatabase::new_at(":memory:");
        let mut build_estate = |house_id: HouseId,
                                is_apartment: bool,
                                name: &str,
                                plot_index: i32,
                                room_number: i32,
                                territory_type_id: i32|
         -> HousingEstate {
            database.insert_housing_estate_for_test(HousingEstate {
                land_ident: house_id.to_u64() as i64,
                house_id: house_id.to_u64() as i64,
                territory_type_id,
                world_id: house_id.world_id as i32,
                ward_index: house_id.ward_index as i32,
                division: 0,
                plot_index,
                room_number,
                is_apartment,
                owner_content_id: Some(100),
                owner_name: "House Owner".to_string(),
                estate_name: name.to_string(),
                ..Default::default()
            })
        };

        let apartment = build_estate(
            HouseId {
                unit: HouseUnit {
                    apartment_division_plot_index: 3,
                    apartment_flag: true,
                },
                unk1: 0,
                ward_index: 2,
                room_number: 2,
                territory_type_id: 340,
                world_id: 21,
            },
            true,
            "Apartment",
            3,
            2,
            340,
        );

        let earlier_non_apartment = build_estate(
            HouseId {
                unit: HouseUnit {
                    apartment_division_plot_index: 1,
                    apartment_flag: false,
                },
                unk1: 0,
                ward_index: 2,
                room_number: 0,
                territory_type_id: 340,
                world_id: 21,
            },
            false,
            "House 1/0",
            1,
            0,
            340,
        );
        let same_plot_roomed = build_estate(
            HouseId {
                unit: HouseUnit {
                    apartment_division_plot_index: 2,
                    apartment_flag: false,
                },
                unk1: 0,
                ward_index: 2,
                room_number: 3,
                territory_type_id: 340,
                world_id: 21,
            },
            false,
            "House 2/3",
            2,
            3,
            340,
        );
        let higher_plot = build_estate(
            HouseId {
                unit: HouseUnit {
                    apartment_division_plot_index: 4,
                    apartment_flag: false,
                },
                unk1: 0,
                ward_index: 2,
                room_number: 0,
                territory_type_id: 340,
                world_id: 21,
            },
            false,
            "House 4/0",
            4,
            0,
            340,
        );

        let other_territory_non_owner = database.insert_housing_estate_for_test(HousingEstate {
            land_ident: HouseId {
                unit: HouseUnit {
                    apartment_division_plot_index: 1,
                    apartment_flag: false,
                },
                unk1: 0,
                ward_index: 2,
                room_number: 0,
                territory_type_id: 341,
                world_id: 21,
            }
            .to_u64() as i64,
            house_id: HouseId {
                unit: HouseUnit {
                    apartment_division_plot_index: 1,
                    apartment_flag: false,
                },
                unk1: 0,
                ward_index: 2,
                room_number: 0,
                territory_type_id: 341,
                world_id: 21,
            }
            .to_u64() as i64,
            territory_type_id: 341,
            world_id: 21,
            ward_index: 2,
            division: 0,
            plot_index: 1,
            room_number: 0,
            is_apartment: false,
            owner_content_id: Some(200),
            owner_name: "Other Owner".to_string(),
            estate_name: "Foreign".to_string(),
            ..Default::default()
        });

        let _ = other_territory_non_owner;

        let ordered_owned = database.owned_housing_estates(100);
        assert_eq!(
            ordered_owned[0].land_ident,
            earlier_non_apartment.land_ident
        );
        assert_eq!(ordered_owned[1].land_ident, same_plot_roomed.land_ident);
        assert_eq!(ordered_owned[2].land_ident, higher_plot.land_ident);
        assert_eq!(ordered_owned[3].land_ident, apartment.land_ident);

        let selected = selected_or_owned_housing_estate(&mut database, None, 100).unwrap();

        assert!(!selected.is_apartment);
        assert_eq!(selected.land_ident, earlier_non_apartment.land_ident);
    }

    #[test]
    fn mutator_selection_updates_active_estate_when_multiple_estates_exist() {
        let mut database = WorldDatabase::new_at(":memory:");
        let default_estate = database.ensure_local_estate(100, "Tester", 67);
        let selected_estate = database.ensure_local_estate_with_spec(HousingEstateSpec {
            owner_content_id: 100,
            owner_name: "Tester FC".to_string(),
            world_id: 67,
            territory_type_id: 341,
            ward_index: 2,
            division: 0,
            plot_index: 12,
            plot_size: PlotSize::Medium,
            free_company: true,
        });
        let active_estate = active_housing_estate(&selected_estate, false);

        let target =
            selected_or_owned_housing_estate(&mut database, Some(&active_estate), 100).unwrap();
        assert!(database.update_housing_name(target.land_ident, "Selected Estate Name"));
        assert!(database.update_housing_greeting(target.land_ident, "Selected estate greeting."));

        let updated_selected = database
            .housing_estate_by_house_id(HouseId::from_u64(selected_estate.house_id as u64))
            .unwrap();
        let updated_default = database
            .housing_estate_by_house_id(HouseId::from_u64(default_estate.house_id as u64))
            .unwrap();

        assert_eq!(updated_selected.estate_name, "Selected Estate Name");
        assert_eq!(updated_selected.greeting, "Selected estate greeting.");
        assert_ne!(updated_default.estate_name, "Selected Estate Name");
        assert_ne!(updated_default.greeting, "Selected estate greeting.");
    }

    #[test]
    fn mutator_selection_ignores_foreign_active_estate_when_updating_owned_estate() {
        let mut database = WorldDatabase::new_at(":memory:");
        let owned_estate = database.ensure_local_estate(100, "Tester", 67);
        let foreign_estate = database.ensure_local_estate_with_spec(HousingEstateSpec {
            owner_content_id: 200,
            owner_name: "Other Owner".to_string(),
            world_id: 67,
            territory_type_id: 341,
            ward_index: 2,
            division: 0,
            plot_index: 12,
            plot_size: PlotSize::Medium,
            free_company: false,
        });
        let active_estate = active_housing_estate(&foreign_estate, false);

        let target =
            selected_or_owned_housing_estate(&mut database, Some(&active_estate), 100).unwrap();
        assert!(database.update_housing_name(target.land_ident, "Owned Estate Name"));
        assert!(database.update_housing_greeting(target.land_ident, "Owned estate greeting."));

        let updated_owned = database
            .housing_estate_by_house_id(HouseId::from_u64(owned_estate.house_id as u64))
            .unwrap();
        let updated_foreign = database
            .housing_estate_by_house_id(HouseId::from_u64(foreign_estate.house_id as u64))
            .unwrap();

        assert_eq!(updated_owned.estate_name, "Owned Estate Name");
        assert_eq!(updated_owned.greeting, "Owned estate greeting.");
        assert_ne!(updated_foreign.estate_name, "Owned Estate Name");
        assert_ne!(updated_foreign.greeting, "Owned estate greeting.");
    }

    #[test]
    fn local_house_simple_interior_territory_matches_plot_size() {
        assert_eq!(
            simple_housing_indoor_territory_type_id(PlotSize::Small),
            1249
        );
        assert_eq!(
            simple_housing_indoor_territory_type_id(PlotSize::Medium),
            1250
        );
        assert_eq!(
            simple_housing_indoor_territory_type_id(PlotSize::Large),
            1251
        );
    }

    #[test]
    fn original_house_interior_territory_matches_retail_district_and_plot_size() {
        assert_eq!(
            district_default_indoor_territory_type_id(340, PlotSize::Large),
            Some(344)
        );
        assert_eq!(
            district_default_indoor_territory_type_id(340, PlotSize::Medium),
            Some(343)
        );
        assert_eq!(
            district_default_indoor_territory_type_id(340, PlotSize::Small),
            Some(342)
        );
    }

    #[test]
    fn default_house_entry_territory_uses_original_district_when_no_pattern_is_saved() {
        let estate = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);

        assert_eq!(
            housing_default_indoor_entry_territory_type_id_for_estate(&estate),
            344
        );
    }

    #[test]
    fn persisted_simple_interior_pattern_row_keeps_simple_territory() {
        let mut estate = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        estate.plot_size = PlotSize::Large as i32;
        estate.interior_json = update_interior_json_renovation_row_id("{}", 18)
            .expect("interior pattern row should serialize");

        assert_eq!(
            housing_interior_renovation_row_id_from_json(&estate.interior_json),
            Some(18)
        );
    }
}
