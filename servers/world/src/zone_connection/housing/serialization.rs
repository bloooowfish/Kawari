use glam::Vec3;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::inventory::{
    HousingInventory, Item, housing_container_slot_capacity, interior_placed_containers,
    interior_storeroom_containers,
};
use crate::{
    HousingEstate, HousingFurniture,
    lua::{HousingExteriorColorField, HousingExteriorField, HousingInteriorField},
};
use kawari::{
    common::{ContainerType, HouseId, HousingFlag, ITEM_CONDITION_MAX, LandData, Position},
    ipc::zone::{
        ApartmentListEntry, AvailabilityType, Furniture, FurnitureList, House, HouseExterior,
        HouseExteriorColors, HouseStatus, HousingEstateGreeting, HousingFlags,
        HousingInteriorDetails, HousingOccupiedLandInfo, HousingVacantLandInfo, HousingWardInfo,
        HousingWardSummaryItem, PlotSize, PurchaseType, TenantType,
    },
};

pub(super) const FREE_COMPANY_HOUSING_FLAG: i32 = 0x10;
pub(super) const MINIMAL_INTERIOR_WALL: u32 = 66111; // Pure White Interior Wall, Item.AdditionalData
pub(super) const MINIMAL_INTERIOR_FLOOR: u32 = 65591; // Light Wood Flooring, Item.AdditionalData
pub(super) const MINIMAL_INTERIOR_LIGHT: u32 = 65848; // Flat Ceiling Lamp, Item.AdditionalData

pub(super) fn owned_housing_land_data(estates: &[HousingEstate]) -> ([LandData; 5], LandData) {
    const FREE_COMPANY_ESTATE_SLOT: usize = 0;
    const PERSONAL_ESTATE_SLOT: usize = 1;

    let mut owned = [LandData::default(); 5];
    let mut apartment = LandData::default();

    for estate in estates {
        let land_data = estate_land_data(estate);

        if estate.is_apartment {
            apartment = land_data;
            continue;
        }

        let target_slot = if estate.flags & FREE_COMPANY_HOUSING_FLAG != 0 {
            FREE_COMPANY_ESTATE_SLOT
        } else {
            PERSONAL_ESTATE_SLOT
        };

        if let Some(slot) = owned.get_mut(target_slot) {
            *slot = land_data;
        }
    }

    (owned, apartment)
}

pub(super) fn build_apartment_list_entries(
    apartments: &[HousingEstate],
    starting_index: u32,
) -> Vec<ApartmentListEntry> {
    let skip = starting_index.saturating_sub(1) as usize;
    let mut apartments = apartments
        .iter()
        .filter(|estate| estate.is_apartment && estate.room_number > 0)
        .cloned()
        .collect::<Vec<_>>();
    apartments.sort_by_key(|estate| (estate.room_number, estate.land_ident));

    apartments
        .into_iter()
        .skip(skip)
        .take(ApartmentListEntry::COUNT)
        .map(|estate| ApartmentListEntry {
            resident_zone_id: estate.territory_type_id.clamp(0, u16::MAX as i32) as u16,
            visitors_permitted: estate.flags & (HousingFlag::OPEN.bits() as i32) != 0,
            resident_name: estate.owner_name,
            apartment_description: if estate.greeting.is_empty() {
                "A local Kawari debug apartment.".to_string()
            } else {
                estate.greeting
            },
            ..Default::default()
        })
        .collect()
}

pub(super) fn estate_land_data(estate: &HousingEstate) -> LandData {
    LandData {
        id: HouseId::from_u64(estate.house_id as u64),
        flags: estate.flags as u32,
        unk1: 0,
    }
}

pub(super) fn build_housing_ward_info(
    territory_type_id: u16,
    world_id: u16,
    ward_index: u8,
    main_estates: &[HousingEstate],
    subdivision_estates: &[HousingEstate],
) -> HousingWardInfo {
    const MAIN_PLOTS: usize = 30;
    const TOTAL_PLOTS: usize = 60;

    let mut house_summaries = vec![HousingWardSummaryItem::default(); TOTAL_PLOTS];

    for estate in main_estates {
        let Ok(plot_index) = usize::try_from(estate.plot_index) else {
            continue;
        };
        let Some(slot) = house_summaries.get_mut(plot_index) else {
            continue;
        };

        *slot = housing_ward_summary_from_estate(estate);
    }

    for estate in subdivision_estates {
        let Ok(plot_index) = usize::try_from(estate.plot_index) else {
            continue;
        };
        let Some(slot) = house_summaries.get_mut(MAIN_PLOTS + plot_index) else {
            continue;
        };

        *slot = housing_ward_summary_from_estate(estate);
    }

    HousingWardInfo {
        id: HouseId {
            ward_index,
            territory_type_id,
            world_id,
            ..Default::default()
        },
        house_summaries,
        purchase_type: PurchaseType::Unavailable,
        tenant_type: TenantType::Any,
        ..Default::default()
    }
}

fn housing_ward_summary_from_estate(estate: &HousingEstate) -> HousingWardSummaryItem {
    HousingWardSummaryItem {
        flags: housing_flags_from_land_flags(estate.flags),
        name: estate.estate_name.clone(),
        ..Default::default()
    }
}

pub(super) fn housing_occupied_land_info_from_estate(
    estate: &HousingEstate,
) -> HousingOccupiedLandInfo {
    HousingOccupiedLandInfo {
        id: HouseId::from_u64(estate.house_id as u64),
        owner_id: estate.owner_content_id.unwrap_or_default() as u64,
        house_icon: housing_house_icon_from_land_flags(estate.flags),
        house_size: PlotSize::from_repr(estate.plot_size as u8).unwrap_or_default(),
        estate_name: estate.estate_name.clone(),
        estate_greeting: estate.greeting.clone(),
        owner_name: estate.owner_name.clone(),
        fc_tag: String::new(),
        ..Default::default()
    }
}

pub(super) fn housing_vacant_land_info() -> HousingVacantLandInfo {
    let mut unk4 = [0; 20];
    unk4[12..].fill(0xFF);

    HousingVacantLandInfo {
        purchase_type: PurchaseType::Unavailable,
        tenant_type: TenantType::Any,
        availability_type: AvailabilityType::Unavailable,
        unk4,
        ..Default::default()
    }
}

pub(super) fn housing_estate_greeting_from_estate(estate: &HousingEstate) -> HousingEstateGreeting {
    HousingEstateGreeting {
        id: HouseId::from_u64(estate.house_id as u64),
        greeting: estate.greeting.clone(),
        ..Default::default()
    }
}

fn housing_flags_from_land_flags(flags: i32) -> HousingFlags {
    let mut housing_flags = HousingFlags::empty();
    if flags & 0x01 != 0 {
        housing_flags |= HousingFlags::PLOT_OWNED;
    }
    if flags & 0x02 != 0 {
        housing_flags |= HousingFlags::VISITORS_ALLOWED;
    }
    if flags & 0x08 != 0 {
        housing_flags |= HousingFlags::HOUSE_BUILT;
    }
    if flags & FREE_COMPANY_HOUSING_FLAG != 0 {
        housing_flags |= HousingFlags::OWNED_BY_FC;
    }
    housing_flags
}

fn housing_house_icon_from_land_flags(flags: i32) -> u8 {
    if flags & 0x08 == 0 {
        2
    } else if flags & 0x02 != 0 {
        1
    } else {
        0
    }
}

pub(super) fn build_house_list_houses(estates: &[HousingEstate]) -> [House; 30] {
    let mut houses = [House::default(); 30];

    for estate in estates {
        let Ok(plot_index) = usize::try_from(estate.plot_index) else {
            continue;
        };
        let Some(slot) = houses.get_mut(plot_index) else {
            continue;
        };

        *slot = house_from_estate(estate);
    }

    houses
}

fn house_from_estate(estate: &HousingEstate) -> House {
    House {
        plot_size: PlotSize::from_repr(estate.plot_size as u8).unwrap_or_default(),
        status: house_status_from_land_flags(estate.flags),
        flags: house_flags_from_land_flags(estate.flags),
        exterior: house_exterior_from_json(&estate.exterior_json),
        ..Default::default()
    }
}

fn house_status_from_land_flags(flags: i32) -> HouseStatus {
    if flags & 0x08 != 0 {
        HouseStatus::HouseBuilt
    } else if flags & 0x01 != 0 {
        HouseStatus::UnderConstruction
    } else {
        HouseStatus::UpForAuction
    }
}

fn house_flags_from_land_flags(flags: i32) -> HousingFlag {
    let mut house_flags = HousingFlag::LOCKED;
    if flags & 0x02 != 0 {
        house_flags |= HousingFlag::OPEN;
    }
    if flags & FREE_COMPANY_HOUSING_FLAG != 0 {
        house_flags |= HousingFlag::OWNED_BY_FC;
    }
    house_flags
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct HouseExteriorJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) roof_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) walls_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) windows_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) door_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) roof_fixture_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) wall_fixture_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) above_door_banner_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) fence_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) colors: Option<HouseExteriorColorsJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct HouseExteriorColorsJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) roof: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) walls: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) windows: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) door: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) roof_fixture: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) wall_fixture: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) above_door_banner: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) fence: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct HouseInteriorJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) renovation_row_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) window_style: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) door_style: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) door_stain: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ground_walls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ground_floor: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ground_chandelier: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_walls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_floor: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_chandelier: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cellar_walls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cellar_floor: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cellar_chandelier: Option<u32>,
}

pub(super) fn house_exterior_from_json(json: &str) -> HouseExterior {
    let fallback = local_estate_house_exterior();
    let exterior = parse_housing_json_or_default::<HouseExteriorJson>(json, "housing exterior");
    let colors = exterior.colors.unwrap_or_default();

    HouseExterior {
        roof_id: exterior.roof_id.unwrap_or(fallback.roof_id),
        walls_id: exterior.walls_id.unwrap_or(fallback.walls_id),
        windows_id: exterior.windows_id.unwrap_or(fallback.windows_id),
        door_id: exterior.door_id.unwrap_or(fallback.door_id),
        roof_fixture_id: exterior.roof_fixture_id.unwrap_or(fallback.roof_fixture_id),
        wall_fixture_id: exterior.wall_fixture_id.unwrap_or(fallback.wall_fixture_id),
        above_door_banner_id: exterior
            .above_door_banner_id
            .unwrap_or(fallback.above_door_banner_id),
        fence_id: exterior.fence_id.unwrap_or(fallback.fence_id),
        colors: HouseExteriorColors {
            roof: colors.roof.unwrap_or(fallback.colors.roof),
            walls: colors.walls.unwrap_or(fallback.colors.walls),
            windows: colors.windows.unwrap_or(fallback.colors.windows),
            door: colors.door.unwrap_or(fallback.colors.door),
            roof_fixture: colors.roof_fixture.unwrap_or(fallback.colors.roof_fixture),
            wall_fixture: colors.wall_fixture.unwrap_or(fallback.colors.wall_fixture),
            above_door_banner: colors
                .above_door_banner
                .unwrap_or(fallback.colors.above_door_banner),
            fence: colors.fence.unwrap_or(fallback.colors.fence),
        },
    }
}

fn local_estate_house_exterior() -> HouseExterior {
    HouseExterior {
        roof_id: 1081,
        walls_id: 3632,
        windows_id: 2579,
        door_id: 531,
        ..Default::default()
    }
}

pub(super) fn build_furniture_lists(
    house_id: HouseId,
    rows: &[HousingFurniture],
    indoors: bool,
    slot_capacity: Option<usize>,
) -> Vec<FurnitureList> {
    let max_rows = Furniture::COUNT * u8::MAX as usize;
    let max_send_rows = slot_capacity.unwrap_or(max_rows).min(max_rows);
    if rows.len() > max_send_rows {
        tracing::warn!(
            house_id = house_id.to_u64(),
            furniture_count = rows.len(),
            max_rows = max_send_rows,
            "Housing furniture list exceeds packet count cap; extra rows will not be sent"
        );
    }

    let total_slots = slot_capacity.unwrap_or_else(|| rows.len().max(1));
    let list_count = total_slots.div_ceil(Furniture::COUNT).max(1);
    let count = list_count.min(u8::MAX as usize) as u8;

    (0..list_count)
        .take(u8::MAX as usize)
        .map(|index| {
            let start = index * Furniture::COUNT;
            let end = (start + Furniture::COUNT)
                .min(rows.len())
                .min(max_send_rows);
            let chunk = if start < end { &rows[start..end] } else { &[] };
            FurnitureList {
                id: house_id,
                index: index as u8,
                count,
                unk2: furniture_list_slot_count(indoors, slot_capacity, index),
                furniture: chunk.iter().map(furniture_from_row).collect(),
                ..Default::default()
            }
        })
        .collect()
}

fn furniture_from_row(row: &HousingFurniture) -> Furniture {
    Furniture {
        id: row.catalog_id.clamp(0, u16::MAX as i32) as u16,
        id2: 0,
        stain: row.stain.clamp(0, u8::MAX as i32) as u8,
        rotation: row.rotation,
        position: Position(Vec3::new(row.pos_x, row.pos_y, row.pos_z)),
    }
}

fn furniture_list_slot_count(indoors: bool, slot_capacity: Option<usize>, list_index: usize) -> u8 {
    if !indoors {
        return 0;
    }

    let Some(slot_capacity) = slot_capacity else {
        return Furniture::COUNT as u8;
    };

    let start = list_index * Furniture::COUNT;
    slot_capacity.saturating_sub(start).min(Furniture::COUNT) as u8
}

pub(super) fn housing_inventory_from_rows(rows: &[HousingFurniture]) -> HousingInventory {
    let mut inventory = HousingInventory::default();

    for row in rows {
        let Some(container) = housing_container_type_from_i32(row.container_type) else {
            tracing::warn!(
                container_type = row.container_type,
                slot = row.slot,
                item_id = row.item_id,
                "Ignoring housing furniture row with unsupported container"
            );
            continue;
        };

        if row.slot < 0 || row.slot as usize >= housing_container_slot_capacity(container) {
            tracing::warn!(
                container_type = row.container_type,
                slot = row.slot,
                item_id = row.item_id,
                "Ignoring housing furniture row with out-of-range slot"
            );
            continue;
        }

        let Some(slot) = inventory.get_item_mut(container, row.slot as u16) else {
            tracing::warn!(
                ?container,
                slot = row.slot,
                item_id = row.item_id,
                "Unable to restore housing furniture row into inventory"
            );
            continue;
        };

        *slot = Item {
            quantity: 1,
            item_id: row.item_id as u32,
            condition: ITEM_CONDITION_MAX,
            stains: [row.stain as u8, 0],
            ..Default::default()
        };
    }

    inventory
}

pub(super) fn housing_container_type_from_i32(container_type: i32) -> Option<ContainerType> {
    let raw_container_type = container_type as u16;

    if let Some(container) = interior_placed_containers()
        .iter()
        .copied()
        .find(|container| *container as u16 == raw_container_type)
    {
        return Some(container);
    }

    if let Some(container) = interior_storeroom_containers()
        .iter()
        .copied()
        .find(|container| *container as u16 == raw_container_type)
    {
        return Some(container);
    }

    match raw_container_type {
        x if x == ContainerType::HousingExteriorAppearance as u16 => {
            Some(ContainerType::HousingExteriorAppearance)
        }
        x if x == ContainerType::HousingExteriorPlacedItems as u16 => {
            Some(ContainerType::HousingExteriorPlacedItems)
        }
        x if x == ContainerType::HousingInteriorAppearance as u16 => {
            Some(ContainerType::HousingInteriorAppearance)
        }
        x if x == ContainerType::HousingExteriorStoreroom as u16 => {
            Some(ContainerType::HousingExteriorStoreroom)
        }
        _ => None,
    }
}

pub(super) fn housing_interior_details(light_level: u8) -> HousingInteriorDetails {
    let mut details = HousingInteriorDetails::default();
    details.light_level = light_level;
    details.ground_walls = MINIMAL_INTERIOR_WALL;
    details.ground_floor = MINIMAL_INTERIOR_FLOOR;
    details.ground_chandelier = MINIMAL_INTERIOR_LIGHT;
    details.top_walls = MINIMAL_INTERIOR_WALL;
    details.top_floor = MINIMAL_INTERIOR_FLOOR;
    details.top_chandelier = MINIMAL_INTERIOR_LIGHT;
    details.cellar_walls = MINIMAL_INTERIOR_WALL;
    details.cellar_floor = MINIMAL_INTERIOR_FLOOR;
    details.cellar_chandelier = MINIMAL_INTERIOR_LIGHT;
    details.unk_interior = MINIMAL_INTERIOR_LIGHT;
    details
}

fn apartment_housing_interior_details(light_level: u8) -> HousingInteriorDetails {
    let mut details = HousingInteriorDetails::default();
    details.light_level = light_level;
    details.ground_walls = MINIMAL_INTERIOR_WALL;
    details.ground_floor = MINIMAL_INTERIOR_FLOOR;
    details.ground_chandelier = MINIMAL_INTERIOR_LIGHT;
    details
}

pub(super) fn housing_interior_details_from_json(
    json: &str,
    light_level: u8,
    is_apartment: bool,
) -> HousingInteriorDetails {
    let mut details = if is_apartment {
        apartment_housing_interior_details(light_level)
    } else {
        housing_interior_details(light_level)
    };
    let interior = parse_housing_json_or_default::<HouseInteriorJson>(json, "housing interior");

    details.window_style = interior.window_style.unwrap_or(details.window_style);
    details.door_style = interior.door_style.unwrap_or(details.door_style);
    details.door_stain = interior.door_stain.unwrap_or(details.door_stain);
    details.ground_walls = interior.ground_walls.unwrap_or(details.ground_walls);
    details.ground_floor = interior.ground_floor.unwrap_or(details.ground_floor);
    details.ground_chandelier = interior
        .ground_chandelier
        .unwrap_or(details.ground_chandelier);

    if !is_apartment {
        details.top_walls = interior.top_walls.unwrap_or(details.top_walls);
        details.top_floor = interior.top_floor.unwrap_or(details.top_floor);
        details.top_chandelier = interior.top_chandelier.unwrap_or(details.top_chandelier);
        details.cellar_walls = interior.cellar_walls.unwrap_or(details.cellar_walls);
        details.cellar_floor = interior.cellar_floor.unwrap_or(details.cellar_floor);
        details.cellar_chandelier = interior
            .cellar_chandelier
            .unwrap_or(details.cellar_chandelier);
    }

    details
}

fn parse_housing_json_or_default<T>(json: &str, label: &str) -> T
where
    T: DeserializeOwned + Default,
{
    if json.trim().is_empty() {
        return T::default();
    }

    match serde_json::from_str(json) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(target = "kawari_world::housing", %label, %error, "Invalid housing json; using defaults");
            T::default()
        }
    }
}

fn parse_housing_json_for_mutation<T>(json: &str, label: &str) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned + Default,
{
    if json.trim().is_empty() {
        return Ok(T::default());
    }

    match serde_json::from_str(json) {
        Ok(value) => Ok(value),
        Err(error) => {
            tracing::warn!(
                target = "kawari_world::housing",
                %label,
                %error,
                "Invalid housing json; refusing to mutate persisted fixture state"
            );
            Err(error)
        }
    }
}

fn serialize_housing_json<T>(value: &T, label: &str) -> Result<String, serde_json::Error>
where
    T: Serialize,
{
    match serde_json::to_string(value) {
        Ok(json) => Ok(json),
        Err(error) => {
            tracing::warn!(
                target = "kawari_world::housing",
                %label,
                %error,
                "Unable to serialize housing json"
            );
            Err(error)
        }
    }
}

pub(super) fn update_exterior_json_field(
    existing_json: &str,
    field: HousingExteriorField,
    value: u16,
) -> Result<String, serde_json::Error> {
    let mut exterior = parse_housing_json_for_mutation::<HouseExteriorJson>(
        existing_json,
        "housing exterior mutation",
    )?;

    match field {
        HousingExteriorField::Roof => exterior.roof_id = Some(value),
        HousingExteriorField::Walls => exterior.walls_id = Some(value),
        HousingExteriorField::Windows => exterior.windows_id = Some(value),
        HousingExteriorField::Door => exterior.door_id = Some(value),
        HousingExteriorField::RoofFixture => exterior.roof_fixture_id = Some(value),
        HousingExteriorField::WallFixture => exterior.wall_fixture_id = Some(value),
        HousingExteriorField::AboveDoorBanner => exterior.above_door_banner_id = Some(value),
        HousingExteriorField::Fence => exterior.fence_id = Some(value),
    }

    serialize_housing_json(&exterior, "housing exterior mutation")
}

pub(super) fn update_exterior_json_color(
    existing_json: &str,
    field: HousingExteriorColorField,
    value: u8,
) -> Result<String, serde_json::Error> {
    let mut exterior = parse_housing_json_for_mutation::<HouseExteriorJson>(
        existing_json,
        "housing exterior color mutation",
    )?;
    let colors = exterior
        .colors
        .get_or_insert_with(HouseExteriorColorsJson::default);

    match field {
        HousingExteriorColorField::Roof => colors.roof = Some(value),
        HousingExteriorColorField::Walls => colors.walls = Some(value),
        HousingExteriorColorField::Windows => colors.windows = Some(value),
        HousingExteriorColorField::Door => colors.door = Some(value),
        HousingExteriorColorField::RoofFixture => colors.roof_fixture = Some(value),
        HousingExteriorColorField::WallFixture => colors.wall_fixture = Some(value),
        HousingExteriorColorField::AboveDoorBanner => colors.above_door_banner = Some(value),
        HousingExteriorColorField::Fence => colors.fence = Some(value),
    }

    serialize_housing_json(&exterior, "housing exterior color mutation")
}

pub(super) fn update_interior_json_field(
    existing_json: &str,
    field: HousingInteriorField,
    value: u32,
) -> Result<String, serde_json::Error> {
    let mut interior = parse_housing_json_for_mutation::<HouseInteriorJson>(
        existing_json,
        "housing interior mutation",
    )?;

    match field {
        HousingInteriorField::WindowStyle => interior.window_style = Some(value as u16),
        HousingInteriorField::DoorStyle => interior.door_style = Some(value as u16),
        HousingInteriorField::DoorStain => interior.door_stain = Some(value as u16),
        HousingInteriorField::GroundWalls => interior.ground_walls = Some(value),
        HousingInteriorField::GroundFloor => interior.ground_floor = Some(value),
        HousingInteriorField::GroundChandelier => interior.ground_chandelier = Some(value),
        HousingInteriorField::TopWalls => interior.top_walls = Some(value),
        HousingInteriorField::TopFloor => interior.top_floor = Some(value),
        HousingInteriorField::TopChandelier => interior.top_chandelier = Some(value),
        HousingInteriorField::CellarWalls => interior.cellar_walls = Some(value),
        HousingInteriorField::CellarFloor => interior.cellar_floor = Some(value),
        HousingInteriorField::CellarChandelier => interior.cellar_chandelier = Some(value),
    }

    serialize_housing_json(&interior, "housing interior mutation")
}

pub(super) fn housing_interior_renovation_row_id_from_json(json: &str) -> Option<u16> {
    parse_housing_json_or_default::<HouseInteriorJson>(json, "housing interior").renovation_row_id
}

pub(super) fn update_interior_json_renovation_row_id(
    existing_json: &str,
    renovation_row_id: u16,
) -> Result<String, serde_json::Error> {
    let mut interior = parse_housing_json_for_mutation::<HouseInteriorJson>(
        existing_json,
        "housing interior renovation mutation",
    )?;
    interior.renovation_row_id = Some(renovation_row_id);

    serialize_housing_json(&interior, "housing interior renovation mutation")
}
