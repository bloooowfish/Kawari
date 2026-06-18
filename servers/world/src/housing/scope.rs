use crate::{HousingEstate, common::HousingFurnitureObjectScope};

const HOUSING_PLOTS_PER_DIVISION: i32 = 30;

pub fn housing_furniture_object_scopes_for_estate(
    estate: &HousingEstate,
) -> Vec<HousingFurnitureObjectScope> {
    if estate.is_apartment {
        return vec![housing_furniture_object_scope_for_estate(estate, true)];
    }

    vec![
        housing_furniture_object_scope_for_estate(estate, false),
        housing_furniture_object_scope_for_estate(estate, true),
    ]
}

pub fn housing_furniture_object_scope_for_estate(
    estate: &HousingEstate,
    indoors: bool,
) -> HousingFurnitureObjectScope {
    HousingFurnitureObjectScope {
        territory_type_id: estate.territory_type_id as u16,
        world_id: estate.world_id as u16,
        ward_index: estate.ward_index as u8,
        division: estate.division as u8,
        indoors,
        plot_index: if indoors {
            0
        } else {
            housing_raw_plot_index_from_estate(estate).unwrap_or_default()
        },
    }
}

pub fn housing_raw_plot_index_from_estate(estate: &HousingEstate) -> Option<u8> {
    let division = estate.division;
    let plot_index = estate.plot_index;
    if !(0..=1).contains(&division) || !(0..HOUSING_PLOTS_PER_DIVISION).contains(&plot_index) {
        return None;
    }

    u8::try_from(plot_index + (division * HOUSING_PLOTS_PER_DIVISION)).ok()
}

pub fn merge_housing_furniture_object_scopes(
    scopes: impl IntoIterator<Item = HousingFurnitureObjectScope>,
) -> Vec<HousingFurnitureObjectScope> {
    let mut merged = Vec::new();
    for scope in scopes {
        if !merged.contains(&scope) {
            merged.push(scope);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawari::common::{HouseId, HouseUnit};

    fn house_id(raw_plot_index: u8) -> i64 {
        HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: raw_plot_index,
                apartment_flag: false,
            },
            ward_index: 2,
            room_number: 0,
            territory_type_id: 340,
            world_id: 21,
            ..Default::default()
        }
        .to_u64() as i64
    }

    #[test]
    fn outdoor_scope_uses_location_columns_instead_of_stale_house_id() {
        let estate = HousingEstate {
            house_id: house_id(5),
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 1,
            plot_index: 5,
            ..Default::default()
        };

        let scope = housing_furniture_object_scope_for_estate(&estate, false);

        assert_eq!(scope.ward_index, 2);
        assert_eq!(scope.division, 1);
        assert_eq!(scope.plot_index, 35);
    }

    #[test]
    fn merge_housing_furniture_object_scopes_deduplicates_ordered_scopes() {
        let scope = HousingFurnitureObjectScope {
            territory_type_id: 340,
            world_id: 21,
            ward_index: 2,
            division: 0,
            indoors: false,
            plot_index: 5,
        };

        assert_eq!(
            merge_housing_furniture_object_scopes([scope, scope]),
            vec![scope]
        );
    }
}
