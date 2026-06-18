use kawari::common::ContainerType;

pub fn container_type_to_i32(container_type: ContainerType) -> i32 {
    container_type as u16 as i32
}

pub fn housing_container_kind(container_type: i32) -> &'static str {
    match container_type {
        value if value == container_type_to_i32(ContainerType::HousingExteriorPlacedItems) => {
            "outdoor_placed"
        }
        value if value == container_type_to_i32(ContainerType::HousingExteriorStoreroom) => {
            "outdoor_storeroom"
        }
        value
            if (container_type_to_i32(ContainerType::HousingInteriorPlacedItems1)
                ..=container_type_to_i32(ContainerType::HousingInteriorPlacedItems12))
                .contains(&value) =>
        {
            "indoor_placed"
        }
        value
            if (container_type_to_i32(ContainerType::HousingInteriorStoreroom1)
                ..=container_type_to_i32(ContainerType::HousingInteriorStoreroom11))
                .contains(&value) =>
        {
            "indoor_storeroom"
        }
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_type_conversion_classifies_housing_kinds() {
        assert_eq!(
            container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            ContainerType::HousingInteriorPlacedItems1 as u16 as i32
        );
        assert_eq!(
            housing_container_kind(container_type_to_i32(
                ContainerType::HousingExteriorPlacedItems
            )),
            "outdoor_placed"
        );
        assert_eq!(
            housing_container_kind(container_type_to_i32(
                ContainerType::HousingExteriorStoreroom
            )),
            "outdoor_storeroom"
        );
        assert_eq!(
            housing_container_kind(container_type_to_i32(
                ContainerType::HousingInteriorPlacedItems12
            )),
            "indoor_placed"
        );
        assert_eq!(
            housing_container_kind(container_type_to_i32(
                ContainerType::HousingInteriorStoreroom11
            )),
            "indoor_storeroom"
        );
        assert_eq!(
            housing_container_kind(container_type_to_i32(ContainerType::Inventory0)),
            "other"
        );
    }
}
