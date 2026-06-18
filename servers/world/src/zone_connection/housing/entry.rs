use super::*;

impl ZoneConnection {
    pub async fn enter_local_house(&mut self) {
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

        let entry = housing_indoor_entry_transform(false);
        let indoor_territory_type_id = self.housing_indoor_territory_type_id_for_estate(&estate);
        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = estate.land_ident,
            house_id = estate.house_id,
            territory_type_id = indoor_territory_type_id,
            "Entering local house interior"
        );

        self.change_zone(
            indoor_territory_type_id,
            Some(entry.position),
            Some(entry.rotation),
            None,
        )
        .await;
    }

    pub async fn enter_local_apartment(&mut self, room_number: u16) {
        if !valid_apartment_room_number(room_number) {
            self.send_notice(&format!(
                "Apartment room numbers must be between 1 and {MAX_APARTMENT_ROOM_NUMBER}."
            ))
            .await;
            return;
        }

        let context = self.apartment_ward_context_or_default();
        let Some(estate) = ({
            let mut database = self.database.lock();
            database.ensure_local_apartment(
                self.player_data.character.content_id as u64,
                &self.player_data.character.name,
                self.config.world_id,
                context.territory_type_id,
                context.ward_index,
                context.division,
                room_number,
            )
        }) else {
            self.send_notice(&format!(
                "Apartment room numbers must be between 1 and {MAX_APARTMENT_ROOM_NUMBER}."
            ))
            .await;
            return;
        };

        self.set_active_housing_estate_from_row(&estate, true);

        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = estate.land_ident,
            house_id = estate.house_id,
            territory_type_id = DEFAULT_LOCAL_APARTMENT_INDOOR_TERRITORY_TYPE_ID,
            room_number,
            "Entering local apartment interior"
        );

        let entry = housing_indoor_entry_transform(true);
        self.change_zone(
            DEFAULT_LOCAL_APARTMENT_INDOOR_TERRITORY_TYPE_ID,
            Some(entry.position),
            Some(entry.rotation),
            None,
        )
        .await;
    }

    pub async fn exit_local_house(&mut self) {
        let active_house_id = self
            .active_housing_estate
            .as_ref()
            .map(|estate| estate.house_id);

        let estate = {
            let mut database = self.database.lock();

            active_house_id
                .and_then(|house_id| database.housing_estate_by_house_id(house_id))
                .or_else(|| {
                    database
                        .owned_housing_estates(self.player_data.character.content_id as u64)
                        .into_iter()
                        .next()
                })
        };

        let Some(estate) = estate else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                "Unable to resolve a housing estate while exiting local house; falling back to New Gridania"
            );
            self.send_notice("Unable to resolve your local estate; falling back to New Gridania.")
                .await;
            self.warp_aetheryte(2, false, false).await;
            return;
        };

        let position = housing_outdoor_exit_fallback_position(&estate);
        let rotation = housing_outdoor_exit_fallback_rotation(&estate);
        let plot_location = housing_outdoor_exit_plot_location(&estate);
        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = estate.land_ident,
            territory_type_id = estate.territory_type_id,
            raw_plot_index = plot_location.map(|location| location.raw_plot_index),
            "Exiting local house to housing outdoor territory"
        );

        if let Some(plot_location) = plot_location {
            self.change_zone_to_housing_plot(plot_location, position, rotation, None)
                .await;
        } else {
            self.change_zone(
                estate.territory_type_id as u16,
                Some(position),
                Some(rotation),
                None,
            )
            .await;
        }
    }
}

pub(super) fn selected_or_default_local_estate(
    database: &mut WorldDatabase,
    active_estate: Option<&ActiveHousingEstate>,
    owner_content_id: u64,
    owner_name: &str,
    world_id: u16,
) -> HousingEstate {
    selected_or_owned_housing_estate(database, active_estate, owner_content_id)
        .unwrap_or_else(|| database.ensure_local_estate(owner_content_id, owner_name, world_id))
}

fn housing_outdoor_exit_fallback_position(_estate: &HousingEstate) -> Position {
    Position(Vec3::new(140.0, 23.5, -0.83))
}

fn housing_outdoor_exit_fallback_rotation(_estate: &HousingEstate) -> f32 {
    0.0
}

fn housing_outdoor_exit_plot_location(estate: &HousingEstate) -> Option<HousingPlotLocation> {
    if estate.is_apartment || estate.room_number != 0 {
        return None;
    }

    let location = outdoor_housing_location_from_estate(estate)?;
    Some(HousingPlotLocation {
        territory_type_id: location.territory_type_id,
        ward_index: location.ward_index,
        raw_plot_index: location.raw_plot_index,
    })
}

pub(super) fn housing_indoor_login_exit_location(
    intended_use: TerritoryIntendedUse,
    estate: Option<&HousingEstate>,
) -> Option<HousingLoginExitLocation> {
    if intended_use != TerritoryIntendedUse::HousingIndoor {
        return None;
    }

    let estate = estate.filter(|estate| !estate.is_apartment)?;
    Some(HousingLoginExitLocation {
        zone_id: estate.territory_type_id.clamp(0, u16::MAX as i32) as u16,
        position: housing_outdoor_exit_fallback_position(estate),
        rotation: housing_outdoor_exit_fallback_rotation(estate),
        plot_location: housing_outdoor_exit_plot_location(estate),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEFAULT_LOCAL_HOUSING_LAND_FLAGS, HousingEstateSpec, WorldDatabase};
    use kawari::common::HouseUnit;

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
    fn housing_outdoor_exit_fallback_position_matches_large_plot_front_door() {
        let position =
            housing_outdoor_exit_fallback_position(&estate(house_id(5, 0, false), 0x0B, false));

        assert_eq!(position, Position(Vec3::new(140.0, 23.5, -0.83)));
    }

    #[test]
    fn housing_outdoor_exit_fallback_rotation_matches_large_plot_front_door() {
        let rotation =
            housing_outdoor_exit_fallback_rotation(&estate(house_id(5, 0, false), 0x0B, false));

        assert_eq!(rotation, 0.0);
    }

    #[test]
    fn housing_outdoor_exit_plot_location_uses_estate_raw_landset_entry() {
        let main = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        let subdivision = ward_estate(5, 1, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);

        let main_location = housing_outdoor_exit_plot_location(&main)
            .expect("main division estate should resolve to a plot entrance request");
        let subdivision_location = housing_outdoor_exit_plot_location(&subdivision)
            .expect("subdivision estate should resolve to a plot entrance request");

        assert_eq!(main_location.territory_type_id, 340);
        assert_eq!(main_location.raw_plot_index, 5);
        assert_eq!(subdivision_location.territory_type_id, 340);
        assert_eq!(subdivision_location.raw_plot_index, 35);
    }

    #[test]
    fn housing_outdoor_exit_plot_location_rejects_apartments_and_invalid_plots() {
        let mut apartment = ward_estate(0, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        apartment.is_apartment = true;
        apartment.room_number = 1;

        let invalid_division = ward_estate(5, 2, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        let invalid_plot = ward_estate(30, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);

        assert_eq!(housing_outdoor_exit_plot_location(&apartment), None);
        assert_eq!(housing_outdoor_exit_plot_location(&invalid_division), None);
        assert_eq!(housing_outdoor_exit_plot_location(&invalid_plot), None);
    }

    #[test]
    fn housing_indoor_login_exit_location_moves_house_to_outdoor_front_door() {
        let house = ward_estate(5, 0, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);

        let location =
            housing_indoor_login_exit_location(TerritoryIntendedUse::HousingIndoor, Some(&house))
                .expect("housing indoor login should be normalized to the outside of the house");

        assert_eq!(location.zone_id, 340);
        assert_eq!(location.position, Position(Vec3::new(140.0, 23.5, -0.83)));
        assert_eq!(location.rotation, 0.0);
        assert_eq!(
            location.plot_location,
            Some(HousingPlotLocation {
                territory_type_id: 340,
                ward_index: 2,
                raw_plot_index: 5,
            })
        );
    }

    #[test]
    fn housing_indoor_login_exit_location_preserves_subdivision_raw_plot_location() {
        let house = ward_estate(5, 1, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);

        let location =
            housing_indoor_login_exit_location(TerritoryIntendedUse::HousingIndoor, Some(&house))
                .expect("housing indoor login should be normalized to the outside of the house");

        assert_eq!(location.zone_id, 340);
        assert_eq!(location.position, Position(Vec3::new(140.0, 23.5, -0.83)));
        assert_eq!(location.rotation, 0.0);
        assert_eq!(
            location.plot_location,
            Some(HousingPlotLocation {
                territory_type_id: 340,
                ward_index: 2,
                raw_plot_index: 35,
            })
        );
    }

    #[test]
    fn housing_indoor_login_exit_location_leaves_non_housing_indoor_zone_alone() {
        let house = estate(house_id(5, 0, false), 0x0B, false);

        assert_eq!(
            housing_indoor_login_exit_location(TerritoryIntendedUse::HousingOutdoor, Some(&house)),
            None
        );
    }

    #[test]
    fn selected_or_default_local_estate_enters_active_parameterized_estate() {
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

        let resolved = selected_or_default_local_estate(
            &mut database,
            Some(&active_estate),
            100,
            "Tester",
            67,
        );

        assert_eq!(resolved.land_ident, selected_estate.land_ident);
        assert_ne!(resolved.land_ident, default_estate.land_ident);
        assert_eq!(database.owned_housing_estates(100).len(), 2);
    }

    #[test]
    fn selected_or_default_local_estate_creates_default_when_only_active_estate_is_foreign() {
        let mut database = WorldDatabase::new_at(":memory:");
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

        let resolved = selected_or_default_local_estate(
            &mut database,
            Some(&active_estate),
            100,
            "Tester",
            67,
        );

        assert_ne!(resolved.land_ident, foreign_estate.land_ident);
        assert_eq!(resolved.owner_content_id, Some(100));
        assert_eq!(database.owned_housing_estates(100).len(), 1);
        assert_eq!(database.owned_housing_estates(200).len(), 1);
    }
}
