use binrw::binrw;

use crate::common::ContainerType;

#[binrw]
#[derive(Debug, Clone, Default)]
pub struct HousingItemOperation {
    pub raw: [u16; 24],
}

impl HousingItemOperation {
    pub fn source_container(&self) -> Option<ContainerType> {
        let (_, container, _) = self.source_hint()?;
        match container {
            0 => Some(ContainerType::Inventory0),
            1 => Some(ContainerType::Inventory1),
            2 => Some(ContainerType::Inventory2),
            3 => Some(ContainerType::Inventory3),
            _ => None,
        }
    }

    pub fn source_slot(&self) -> Option<u16> {
        let (_, _, slot) = self.source_hint()?;
        Some(slot)
    }

    pub fn target_appearance_slot(&self) -> Option<u16> {
        let (target_slot, _, _) = self.source_hint()?;
        Some(target_slot)
    }

    fn source_hint(&self) -> Option<(u16, u16, u16)> {
        (0..10).find_map(|target_slot| {
            let container_index = 4 + target_slot;
            let slot_index = 14 + target_slot;
            self.raw
                .get(container_index)
                .zip(self.raw.get(slot_index))
                .and_then(|(&container, &slot)| {
                    (container <= 3 && slot != u16::MAX).then_some((
                        target_slot as u16,
                        container,
                        slot,
                    ))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::BinRead;

    use crate::common::ContainerType;

    use super::*;

    #[test]
    fn read_retail_housing_item_operation_marker() {
        let bytes = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27,
            0x0F, 0x27, 0x0F, 0x27, 0x00, 0x00, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0x00, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ];

        let operation = HousingItemOperation::read_le(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(
            operation.source_container(),
            Some(ContainerType::Inventory0)
        );
        assert_eq!(operation.source_slot(), Some(2));
    }

    #[test]
    fn read_local_housing_item_operation_marker_with_shifted_source_hint() {
        let operation = HousingItemOperation {
            raw: [
                0, 0, 0, 0, 9999, 9999, 0, 9999, 9999, 9999, 9999, 9999, 9999, 9999, 65535, 65535,
                0, 65535, 65535, 65535, 65535, 65535, 65535, 65535,
            ],
        };

        assert_eq!(
            operation.source_container(),
            Some(ContainerType::Inventory0)
        );
        assert_eq!(operation.source_slot(), Some(0));
    }

    #[test]
    fn local_housing_item_operation_marker_exposes_target_appearance_slot() {
        let operation = HousingItemOperation {
            raw: [
                0, 0, 0, 0, 9999, 9999, 0, 9999, 9999, 9999, 9999, 9999, 9999, 9999, 65535, 65535,
                0, 65535, 65535, 65535, 65535, 65535, 65535, 65535,
            ],
        };

        assert_eq!(operation.target_appearance_slot(), Some(2));
    }

    #[test]
    fn retail_housing_item_operation_marker_exposes_target_appearance_slot() {
        let operation = HousingItemOperation {
            raw: [
                0, 0, 0, 0, 9999, 9999, 9999, 9999, 9999, 0, 9999, 9999, 9999, 9999, 65535, 65535,
                65535, 65535, 65535, 2, 65535, 65535, 65535, 65535,
            ],
        };

        assert_eq!(operation.target_appearance_slot(), Some(5));
    }
}
