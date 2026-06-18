use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OutdoorHousingLocation {
    pub(super) territory_type_id: u16,
    pub(super) ward_index: u8,
    pub(super) division: u8,
    pub(super) plot_index: u8,
    pub(super) raw_plot_index: u8,
}

pub(super) const HOUSING_PLOTS_PER_DIVISION: u8 = 30;
pub(super) const APARTMENT_INTERIOR_TERRITORY_TYPE_IDS: [u16; 5] = [608, 609, 610, 655, 999];

impl ZoneConnection {
    pub fn resolve_active_housing_estate(
        &mut self,
        intended_use: TerritoryIntendedUse,
        zone_id: u16,
    ) {
        self.active_housing_estate = match intended_use {
            TerritoryIntendedUse::HousingIndoor => {
                let preferred_apartment_context = Some(self.apartment_ward_context_or_default());
                {
                    let mut database = self.database.lock();
                    resolve_active_indoor_housing_estate(
                        &mut database,
                        self.active_housing_estate.as_ref(),
                        preferred_apartment_context,
                        zone_id,
                        self.config.world_id,
                        self.player_data.character.content_id as u64,
                    )
                }
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

    pub(super) fn active_housing_estate_ward_context(&self) -> Option<ActiveHousingWardContext> {
        let active_estate = self.active_housing_estate.as_ref()?;
        let estate = {
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(active_estate.house_id)
        }?;

        Some(active_housing_ward_context_from_estate(&estate))
    }

    pub(super) fn default_housing_ward_context(&self) -> ActiveHousingWardContext {
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

    pub(crate) fn selected_or_owned_housing_estate(&mut self) -> Option<HousingEstate> {
        let active_estate = self.active_housing_estate.clone();
        let mut database = self.database.lock();
        selected_or_owned_housing_estate(
            &mut database,
            active_estate.as_ref(),
            self.player_data.character.content_id as u64,
        )
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
}

pub(super) fn active_housing_estate(estate: &HousingEstate, indoors: bool) -> ActiveHousingEstate {
    ActiveHousingEstate {
        land_ident: estate.land_ident,
        house_id: HouseId::from_u64(estate.house_id as u64),
        indoors,
    }
}

pub(super) fn selected_or_owned_housing_estate(
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

pub(super) fn resolve_active_indoor_housing_estate(
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

pub(super) fn apartment_interior_zone_id(zone_id: u16) -> bool {
    APARTMENT_INTERIOR_TERRITORY_TYPE_IDS.contains(&zone_id)
}

pub(super) fn housing_estate_plot_size(estate: &HousingEstate) -> PlotSize {
    PlotSize::from_repr(estate.plot_size as u8).unwrap_or(PlotSize::Large)
}

pub(super) fn active_housing_ward_context_from_estate(
    estate: &HousingEstate,
) -> ActiveHousingWardContext {
    ActiveHousingWardContext {
        territory_type_id: estate.territory_type_id.clamp(0, u16::MAX as i32) as u16,
        ward_index: estate.ward_index.clamp(0, u8::MAX as i32) as u8,
        division: estate.division.clamp(0, u8::MAX as i32) as u8,
    }
}

pub(super) fn display_housing_ward_context_or_default(
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

pub(super) fn placard_authoritative_estate(
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

pub(super) fn outdoor_init_display_context(
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

pub(super) fn outdoor_init_active_context(
    active_context: Option<ActiveHousingWardContext>,
    _active_estate_context: Option<ActiveHousingWardContext>,
) -> Option<ActiveHousingWardContext> {
    active_context
}

#[cfg(test)]
pub(super) fn trusted_housing_ward_context_after_display_update(
    active_context: Option<ActiveHousingWardContext>,
    _display_context: ActiveHousingWardContext,
) -> Option<ActiveHousingWardContext> {
    active_context
}

#[cfg(test)]
pub(super) fn trusted_housing_ward_context_after_vacant_placard(
    active_context: Option<ActiveHousingWardContext>,
    _display_context: ActiveHousingWardContext,
) -> Option<ActiveHousingWardContext> {
    active_context
}

pub(super) fn outdoor_init_authoritative_context(
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

pub(super) fn resolve_active_housing_estate_for_outdoor_owner_gate(
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

pub(super) fn resolve_active_housing_estate_for_outdoor_edit(
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

pub(super) fn outdoor_housing_location_from_raw_entry(
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

pub(super) fn active_housing_estate_for_outdoor_row(
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

pub(super) fn active_housing_estate_for_outdoor_removal_row(
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

pub(super) fn housing_estate_matches_outdoor_location(
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

pub(super) fn outdoor_housing_location_from_estate(
    estate: &HousingEstate,
) -> Option<OutdoorHousingLocation> {
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

pub(super) fn outdoor_house_id_from_estate(estate: &HousingEstate) -> HouseId {
    let Some(location) = outdoor_housing_location_from_estate(estate) else {
        return HouseId::from_u64(estate.house_id as u64);
    };

    outdoor_house_id_from_estate_location(estate, location)
}

pub(super) fn outdoor_house_id_from_estate_location(
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

pub(super) fn can_edit_housing_estate(estate: &HousingEstate, content_id: u64) -> bool {
    estate.owner_content_id == Some(content_id as i64)
}
