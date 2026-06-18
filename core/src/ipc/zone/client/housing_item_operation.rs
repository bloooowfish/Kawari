use binrw::binrw;

#[binrw]
#[derive(Debug, Clone, Default)]
pub struct HousingItemOperation {
    pub raw: [u16; 24],
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::BinRead;

    use super::*;

    #[test]
    fn read_retail_housing_item_operation_marker_layout() {
        let bytes = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27,
            0x0F, 0x27, 0x0F, 0x27, 0x00, 0x00, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27, 0x0F, 0x27,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0x00, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ];

        let operation = HousingItemOperation::read_le(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(
            operation.raw,
            [
                0, 0, 0, 0, 9999, 9999, 9999, 9999, 9999, 0, 9999, 9999, 9999, 9999, 65535, 65535,
                65535, 65535, 65535, 2, 65535, 65535, 65535, 65535,
            ]
        );
    }
}
