use physis::TerritoryIntendedUse;

use super::serialization::{
    housing_interior_renovation_row_id_from_json, update_interior_json_renovation_row_id,
};
use super::{
    ActiveHousingEstate, DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_LARGE,
    DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_MEDIUM,
    DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_SMALL, active_housing_estate,
    housing_estate_plot_size, housing_indoor_entry_transform, selected_or_owned_housing_estate,
};
use crate::{HousingEstate, WorldDatabase, zone_connection::ZoneConnection};
use kawari::ipc::zone::PlotSize;

impl ZoneConnection {
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

    pub(super) fn housing_indoor_territory_type_id_for_estate(
        &mut self,
        estate: &HousingEstate,
    ) -> u16 {
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
}

pub(super) fn simple_housing_indoor_territory_type_id(plot_size: PlotSize) -> u16 {
    match plot_size {
        PlotSize::Small => DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_SMALL,
        PlotSize::Medium => DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_MEDIUM,
        PlotSize::Large => DEFAULT_LOCAL_HOUSING_INDOOR_TERRITORY_TYPE_ID_LARGE,
    }
}

pub(super) fn housing_default_indoor_entry_territory_type_id_for_estate(
    estate: &HousingEstate,
) -> u16 {
    let outdoor_territory_type_id = estate.territory_type_id.clamp(0, u16::MAX as i32) as u16;
    district_default_indoor_territory_type_id(
        outdoor_territory_type_id,
        housing_estate_plot_size(estate),
    )
    .unwrap_or_else(|| simple_housing_indoor_territory_type_id(housing_estate_plot_size(estate)))
}

pub(super) fn district_default_indoor_territory_type_id(
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

pub(super) fn active_housing_estate_for_owned_outdoor_pattern(
    database: &mut WorldDatabase,
    active_estate: Option<&ActiveHousingEstate>,
    content_id: u64,
) -> Option<ActiveHousingEstate> {
    selected_or_owned_housing_estate(database, active_estate, content_id)
        .filter(|estate| !estate.is_apartment && estate.room_number == 0)
        .map(|estate| active_housing_estate(&estate, false))
}

pub(super) fn housing_interior_pattern_area_allows_request(
    intended_use: TerritoryIntendedUse,
) -> bool {
    matches!(
        intended_use,
        TerritoryIntendedUse::HousingOutdoor | TerritoryIntendedUse::HousingIndoor
    )
}

pub(super) fn housing_interior_pattern_apply_should_reload(
    intended_use: TerritoryIntendedUse,
) -> bool {
    intended_use == TerritoryIntendedUse::HousingIndoor
}
