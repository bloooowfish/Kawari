use kawari::{common::ContainerType, ipc::zone::HousingItemOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HousingItemOperationHint {
    pub source_container: ContainerType,
    pub source_slot: u16,
    pub target_appearance_slot: u16,
}

pub(super) fn housing_item_operation_hint(
    action: &HousingItemOperation,
) -> Option<HousingItemOperationHint> {
    let (target_appearance_slot, container, source_slot) = source_hint(action)?;
    let source_container = match container {
        0 => ContainerType::Inventory0,
        1 => ContainerType::Inventory1,
        2 => ContainerType::Inventory2,
        3 => ContainerType::Inventory3,
        _ => return None,
    };

    Some(HousingItemOperationHint {
        source_container,
        source_slot,
        target_appearance_slot,
    })
}

fn source_hint(action: &HousingItemOperation) -> Option<(u16, u16, u16)> {
    (0..10).find_map(|target_slot| {
        let container_index = 4 + target_slot;
        let slot_index = 14 + target_slot;
        action
            .raw
            .get(container_index)
            .zip(action.raw.get(slot_index))
            .and_then(|(&container, &slot)| {
                (container <= 3 && slot != u16::MAX).then_some((
                    target_slot as u16,
                    container,
                    slot,
                ))
            })
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::BinRead;
    use kawari::ipc::zone::HousingItemOperation;

    use super::*;

    #[test]
    fn housing_item_operation_hint_reads_retail_marker_source_and_target() {
        let bytes = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27,
            0x0F, 0x27, 0x0F, 0x27, 0x00, 0x00, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0x00, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ];

        let operation = HousingItemOperation::read_le(&mut Cursor::new(bytes)).unwrap();
        let hint = housing_item_operation_hint(&operation).unwrap();

        assert_eq!(hint.source_container, ContainerType::Inventory0);
        assert_eq!(hint.source_slot, 2);
        assert_eq!(hint.target_appearance_slot, 5);
    }

    #[test]
    fn housing_item_operation_hint_reads_shifted_local_marker_source_and_target() {
        let operation = HousingItemOperation {
            raw: [
                0, 0, 0, 0, 9999, 9999, 0, 9999, 9999, 9999, 9999, 9999, 9999, 9999, 65535, 65535,
                0, 65535, 65535, 65535, 65535, 65535, 65535, 65535,
            ],
        };

        let hint = housing_item_operation_hint(&operation).unwrap();

        assert_eq!(hint.source_container, ContainerType::Inventory0);
        assert_eq!(hint.source_slot, 0);
        assert_eq!(hint.target_appearance_slot, 2);
    }

    #[test]
    fn housing_item_operation_hint_ignores_markers_without_usable_slot() {
        let operation = HousingItemOperation {
            raw: [
                0, 0, 0, 0, 0, 9999, 9999, 9999, 9999, 9999, 9999, 9999, 9999, 9999, 65535, 65535,
                65535, 65535, 65535, 65535, 65535, 65535, 65535, 65535,
            ],
        };

        assert!(housing_item_operation_hint(&operation).is_none());
    }
}
