use std::path::PathBuf;

use crate::{inventory::Item, lua::HousingPresetScope};
use kawari::common::{ContainerType, HouseId};

#[derive(Debug, Default, Clone)]
pub struct ActiveHousingEstate {
    pub land_ident: i64,
    pub house_id: HouseId,
    pub indoors: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ActiveHousingWardContext {
    pub territory_type_id: u16,
    pub ward_index: u8,
    pub division: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingHousingAppearanceItemOperation {
    pub source_container: ContainerType,
    pub source_slot: u16,
    pub target_container: ContainerType,
    pub target_slot: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct AppliedHousingAppearanceItemOperation {
    pub source_container: ContainerType,
    pub source_slot: u16,
    pub target_container: ContainerType,
    pub target_slot: u16,
    pub original_source_item: Item,
    pub original_target_item: Item,
}

#[derive(Clone, Debug)]
pub struct LastHousingPreset {
    pub path: PathBuf,
    pub scope: HousingPresetScope,
}
