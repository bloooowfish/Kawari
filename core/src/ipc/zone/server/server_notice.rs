use binrw::binrw;
use bitflags::bitflags;

use crate::common::{read_string, write_string};

#[binrw]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct ServerNoticeFlags(pub u8);

impl std::fmt::Debug for ServerNoticeFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        bitflags::parser::to_writer(self, f)
    }
}

// See https://github.com/SapphireServer/Sapphire/blob/bf3368224a00c180cbb7ba413b52395eba58ec0b/src/common/Network/PacketDef/Zone/ServerZoneDef.h#L250
bitflags! {
    impl ServerNoticeFlags : u8 {
        /// Shows in the chat log.
        const CHAT_LOG = 0x001;
        /// Shows as an on-screen message.
        const ON_SCREEN = 0x004;
    }
}

#[binrw]
#[derive(Debug, Clone, Default)]
pub struct ServerNoticeMessage {
    pub flags: ServerNoticeFlags,
    #[brw(pad_size_to = 775)]
    #[br(count = 775)]
    #[br(map = read_string)]
    #[bw(map = write_string)]
    pub message: String,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::BinWrite;

    use super::super::{ServerZoneIpcData, ServerZoneIpcSegment};
    use super::*;
    use crate::packet::ReadWriteIpcSegment;

    fn preset_summary_like_message() -> String {
        format!(
            "Applied ReMakePlace preset {} to {} ({}): indoor={} outdoor={} fixtures={} style={} replaced={} skipped missing_item={} missing_catalog={} capacity={} fixture_missing_item={} fixture_missing_data={} fixture_wrong_category={}. {}",
            r#"D:\ReMakePlace_Latest\MakePlace\Save\CL03 Meridian Neue L.json"#,
            "Cha Min's Test Estate",
            "interior",
            596,
            0,
            9,
            18,
            596,
            0,
            0,
            0,
            0,
            0,
            0,
            "Use !housing reload or re-enter the estate/ward to refresh visuals.",
        )
    }

    #[test]
    fn server_notice_message_writes_fixed_size_with_preset_summary_text() {
        let message = ServerNoticeMessage {
            flags: ServerNoticeFlags::CHAT_LOG,
            message: preset_summary_like_message(),
        };
        let mut cursor = Cursor::new(Vec::new());

        message.write_le(&mut cursor).unwrap();

        assert_eq!(cursor.into_inner().len(), 776);
    }

    #[test]
    fn server_notice_ipc_writes_expected_size_with_preset_summary_text() {
        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::ServerNoticeMessage(
            ServerNoticeMessage {
                flags: ServerNoticeFlags::CHAT_LOG,
                message: preset_summary_like_message(),
            },
        ));
        let mut cursor = Cursor::new(Vec::new());

        ipc.write_le(&mut cursor).unwrap();

        assert_eq!(cursor.into_inner().len(), ipc.calc_size() as usize);
    }
}
