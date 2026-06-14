use crate::common::{HouseId, Position};
use binrw::binrw;

#[binrw]
#[derive(Clone, Debug, Default)]
pub struct FurnitureList {
    /// The LandId this list is for.
    pub id: HouseId,
    pub unk1: u8,
    /// The current `index` out of `count` packets to be sent.
    pub index: u8,
    /// The number of these lists that will be sent.
    pub count: u8,
    /// Indoor lists use this as the number of furniture slots represented by this packet.
    /// Outdoor lists observed so far keep this at 0.
    pub unk2: u8,
    /// The actual furnishings.
    #[br(count = Furniture::COUNT)]
    #[brw(pad_size_to = Furniture::COUNT * Furniture::SIZE)]
    #[brw(pad_after = 4)] // Seems to be empty/zeroes
    pub furniture: Vec<Furniture>,
}

#[binrw]
#[derive(Copy, Clone, Debug, Default)]
pub struct HousingInteriorDetails {
    /// This interior's window style.
    pub window_style: u16,
    pub unk1: u16, // Sapphire calls this "window color", but windows cannot be dyed?
    /// This interior's door style.
    pub door_style: u16,
    /// This interior door's dye colour. Index into the Stain Excel sheet.
    /// IDA suggests the low byte is the door stain and the high byte may overlap the client-side light transition level.
    /// Keep the existing u16 layout until runtime packet traces prove a safer split.
    pub door_stain: u16,
    /// The client-side current/active light byte for the interior. The adjacent high byte of `door_stain` is the likely
    /// darkness transition level according to IDA; this field is kept as-is for compatibility with current persistence.
    pub light_level: u8,
    pub unk2: [u8; 3], // likely just padding
    /// The ground floor's wall style. In an apartment, this along with ground_floor and ground_chandelier dictate what will decorate the apartment, leaving doors, windows, top floor and cellar all zeroes/blank.
    // TODO: It's unclear if these are pairs of u16s or just u32s.
    // NOTE: Be careful when experimenting with these values, as invalid combinations of u16s can crash the client, particularly if the interior is an apartment and top floor or cellar values are changed!
    pub ground_walls: u32,
    /// The ground floor's style/texture.
    pub ground_floor: u32,
    /// THe ground floor's chandelier. Unknown if this is a model id + toggle, or an item id + toggle.
    pub ground_chandelier: u32,
    /// The top floor's wall style/texture.
    pub top_walls: u32,
    /// The top floor's style/texture.
    pub top_floor: u32,
    /// The top floor's chandelier.
    pub top_chandelier: u32,
    /// The cellar's wall style/texture.
    pub cellar_walls: u32,
    /// The cellar's floor syle/texture.
    pub cellar_floor: u32,
    /// The cellar's chandelier.
    pub cellar_chandelier: u32,
    pub unk_interior: u32, // Unclear what this is, it can have data in mansions but not apartments or medium houses?
    unk3: u32,             // Might just be padding, seen as zeroes so far
}

#[binrw]
#[derive(Copy, Clone, Debug, Default)]
pub struct Furniture {
    /// Index into the FurnitureCatalogItemList sheet. If 0, no item is present in this entry. Therefore, this index needs to subtract 1 when indexing into the sheet!
    pub id: u16,
    /// Unknown.
    pub id2: i16,
    /// Index into the Stain sheet. Sets the dye for this item.
    #[brw(pad_after = 3)] // Empty, not read by the client.
    pub stain: u8,
    /// This item's rotation.
    pub rotation: f32,
    /// This item's 3d coordinates in the housing interior.
    pub position: Position,
}

/// Per-furniture object data map values used by current clients during housing load.
///
/// IDA shows the client stores up to 8 entries per furniture index. Each entry is
/// split across parallel arrays in the packet and expanded client-side into a
/// 6-byte value-set record.
#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct HousingObjectDataValueSet {
    pub furniture_index: u16,
    pub value_count: u8,
    pub reserved: u8,
    pub values: [u16; 8],
    pub param_a: [u8; 8],
    pub param_b: [u8; 8],
    pub param_c: [u8; 8],
    pub padding: [u8; 4],
}

/// Data sent to a client that observes another client moving or rotating furniture.
#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct FurnitureTranslatedForObserver {
    /// This furniture's new rotation.
    pub rotation: f32,
    /// NOTE: The purpose of `plot_and_index` changes depending on whether this operation is happening indoors or outdoors. When indoors, this value stays as a u16 so that furniture can be addressed beyond an index of 255. When outdoors, it's treated as two separate values: `outdoor_index`, and `plot_number`. See their comments below for more info.
    pub plot_and_index: u16,
    /// When outdoors, this byte represents the affected index into the outdoor furniture. This exists mainly for PacketAnalyzer display.
    #[br(calc = (plot_and_index & 0xFF) as u8)]
    #[bw(ignore)]
    pub outdoor_index: u8,
    /// When outdoors, this byte represents which plot the furniture was moved on. This exists mainly for PacketAnalyzer display.
    #[br(calc = (plot_and_index >> 8) as u8)]
    #[bw(ignore)]
    pub plot_number: u8,
    pub unk1: [u8; 2], // Likely just padding, observed as zeroes
    /// This furniture's new position in the world.
    pub position: Position,
    pub unk2: [u8; 4], // Likely just padding, observed as zeroes
}

impl Furniture {
    pub const SIZE: usize = 24;
    pub const COUNT: usize = 100;
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::BinWrite;

    use super::*;

    #[test]
    fn housing_object_data_value_set_writes_retail_layout() {
        let mut buffer = Cursor::new(Vec::new());
        HousingObjectDataValueSet {
            furniture_index: 0x00ce,
            value_count: 8,
            values: [1, 1, 1, 1, 1, 1, 0x0101, 0x0201],
            param_a: [2, 3, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        }
        .write_le(&mut buffer)
        .unwrap();

        let bytes = buffer.into_inner();
        assert_eq!(bytes.len(), 48);
        assert_eq!(&bytes[0..4], &[0xce, 0x00, 0x08, 0x00]);
        assert_eq!(
            &bytes[4..20],
            &[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 2]
        );
        assert_eq!(&bytes[20..28], &[2, 3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&bytes[28..], &[0; 20]);
    }
}
