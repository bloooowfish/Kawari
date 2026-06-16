use binrw::binrw;
use kawari_core_macro::opcode_data;

use crate::{
    common::{
        CHAR_NAME_MAX_LENGTH, ObjectId, read_bool_from, read_string, write_bool_as, write_string,
    },
    ipc::lobby::CharacterDetails,
    opcodes::CustomIpcType,
    packet::{IpcSegment, ServerlessIpcSegmentHeader},
};

pub type CustomIpcSegment =
    IpcSegment<ServerlessIpcSegmentHeader<CustomIpcType>, CustomIpcType, CustomIpcData>;

pub const HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES: usize = 4096;
pub const HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES: usize = 8192;
pub const HOUSING_ADMIN_NAME_MAX_BYTES: usize = 20;
pub const HOUSING_ADMIN_GREETING_MAX_BYTES: usize = 192;
pub const HOUSING_ADMIN_EXPORT_PATH_MAX_BYTES: usize = 260;
pub const HOUSING_ADMIN_IMPORT_PATH_MAX_BYTES: usize = 260;
pub const HOUSING_ADMIN_MESSAGE_MAX_BYTES: usize = 512;
pub const HOUSING_EXPORTS_DIR: &str = "housing-exports";

#[opcode_data(CustomIpcType)]
#[binrw]
#[br(import(magic: &CustomIpcType, size: &u32))]
#[derive(Debug, Clone)]
pub enum CustomIpcData {
    RequestCreateCharacter {
        service_account_id: u64,
        #[bw(pad_size_to = CHAR_NAME_MAX_LENGTH)]
        #[br(count = CHAR_NAME_MAX_LENGTH)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        name: String,
        #[bw(pad_size_to = 1024)]
        #[br(count = 1024)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        chara_make_json: String,
    },
    CharacterCreated {
        actor_id: ObjectId,
        content_id: u64,
    },
    GetActorId {
        content_id: u64,
    },
    ActorIdFound {
        actor_id: ObjectId,
    },
    CheckNameIsAvailable {
        #[bw(pad_size_to = CHAR_NAME_MAX_LENGTH)]
        #[br(count = CHAR_NAME_MAX_LENGTH)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        name: String,
    },
    NameIsAvailableResponse {
        #[br(map = read_bool_from::<u8>)]
        #[bw(map = write_bool_as::<u8>)]
        free: bool,
    },
    RequestCharacterList {
        service_account_id: u64,
    },
    RequestCharacterListResponse {
        #[bw(calc = characters.len() as u8)]
        num_characters: u8,
        #[br(count = num_characters)]
        #[brw(pad_size_to = CharacterDetails::SIZE * 8)]
        characters: Vec<CharacterDetails>,
    },
    DeleteCharacter {
        content_id: u64,
    },
    CharacterDeleted {
        deleted: u8,
    },
    ImportCharacter {
        service_account_id: u64,
        #[bw(pad_size_to = 128)]
        #[br(count = 128)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        path: String,
    },
    RemakeCharacter {
        content_id: u64,
        #[bw(pad_size_to = 1024)]
        #[br(count = 1024)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        chara_make_json: String,
    },
    CharacterRemade {
        content_id: u64,
    },
    CharacterImported {
        #[bw(pad_size_to = 128)]
        #[br(count = 128)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        message: String,
    },
    DeleteServiceAccount {
        service_account_id: u64,
    },
    RequestFullCharacterList {},
    FullCharacterListResponse {
        #[bw(pad_size_to = 1024)]
        #[br(count = 1024)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        json: String,
    },
    RequestHousingSummary {},
    HousingSummaryResponse {
        #[bw(pad_size_to = HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        json: String,
    },
    RequestHousingEstateDetail {
        land_ident: i64,
    },
    HousingEstateDetailResponse {
        #[bw(pad_size_to = HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        json: String,
    },
    ResetHousingFurniture {
        land_ident: i64,
    },
    ResetHousingEstate {
        land_ident: i64,
    },
    UpdateHousingEstateText {
        land_ident: i64,
        #[bw(pad_size_to = HOUSING_ADMIN_NAME_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_NAME_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        name: String,
        #[bw(pad_size_to = HOUSING_ADMIN_GREETING_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_GREETING_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        greeting: String,
    },
    ExportHousingEstate {
        land_ident: i64,
    },
    HousingEstateExported {
        #[bw(pad_size_to = HOUSING_ADMIN_EXPORT_PATH_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_EXPORT_PATH_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        path: String,
        #[bw(pad_size_to = HOUSING_ADMIN_MESSAGE_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_MESSAGE_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        message: String,
    },
    ImportHousingEstate {
        #[bw(pad_size_to = HOUSING_ADMIN_IMPORT_PATH_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_IMPORT_PATH_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        path: String,
    },
    HousingEstateImportResult {
        #[bw(pad_size_to = HOUSING_ADMIN_MESSAGE_MAX_BYTES)]
        #[br(count = HOUSING_ADMIN_MESSAGE_MAX_BYTES)]
        #[br(map = read_string)]
        #[bw(map = write_string)]
        message: String,
    },
}

fn fixed_string_serialized_len(value: &str) -> usize {
    write_string(&value.to_string()).len()
}

pub fn truncate_utf8_to_max_bytes(value: &str, max_bytes: usize) -> String {
    if fixed_string_serialized_len(value) <= max_bytes {
        return value.to_string();
    }

    let mut end = value.len().min(max_bytes);
    while end > 0 {
        if value.is_char_boundary(end) {
            let truncated = &value[..end];
            if fixed_string_serialized_len(truncated) <= max_bytes {
                return truncated.to_string();
            }
        }

        end -= 1;
    }

    String::new()
}

fn overflow_json_payload(error: &str, max_bytes: usize) -> String {
    let payload = format!(r#"{{"error":"{error}","truncated":true}}"#);
    debug_assert!(fixed_string_serialized_len(&payload) <= max_bytes);
    payload
}

pub fn clamp_housing_summary_json_for_ipc(json: &str) -> String {
    if fixed_string_serialized_len(json) <= HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES {
        json.to_string()
    } else {
        overflow_json_payload(
            "housing_summary_ipc_overflow",
            HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES,
        )
    }
}

pub fn clamp_housing_detail_json_for_ipc(json: &str) -> String {
    if fixed_string_serialized_len(json) <= HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES {
        json.to_string()
    } else {
        overflow_json_payload(
            "housing_detail_ipc_overflow",
            HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES,
        )
    }
}

pub fn clamp_housing_admin_name_for_ipc(name: &str) -> String {
    truncate_utf8_to_max_bytes(name, HOUSING_ADMIN_NAME_MAX_BYTES)
}

pub fn clamp_housing_admin_greeting_for_ipc(greeting: &str) -> String {
    truncate_utf8_to_max_bytes(greeting, HOUSING_ADMIN_GREETING_MAX_BYTES)
}

pub fn clamp_housing_export_path_for_ipc(path: &str) -> String {
    truncate_utf8_to_max_bytes(path, HOUSING_ADMIN_EXPORT_PATH_MAX_BYTES)
}

pub fn clamp_housing_message_for_ipc(message: &str) -> String {
    truncate_utf8_to_max_bytes(message, HOUSING_ADMIN_MESSAGE_MAX_BYTES)
}

fn invalid_housing_import_path(message: &str) -> Result<String, String> {
    Err(clamp_housing_message_for_ipc(message))
}

pub fn validate_housing_import_path_for_ipc(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return invalid_housing_import_path("Import path is required.");
    }

    if trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || trimmed.starts_with("//")
        || trimmed.starts_with("\\\\")
        || trimmed
            .as_bytes()
            .get(1)
            .is_some_and(|separator| *separator == b':')
    {
        return invalid_housing_import_path(
            "Import path must stay inside housing-exports and cannot be absolute.",
        );
    }

    let parts = trimmed
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return invalid_housing_import_path("Import path is required.");
    }

    if parts.iter().any(|part| *part == ".." || part.contains(':')) {
        return invalid_housing_import_path(
            "Import path must stay inside housing-exports and cannot contain parent traversal.",
        );
    }

    let relative_parts = if parts.first() == Some(&HOUSING_EXPORTS_DIR) {
        &parts[1..]
    } else if parts.len() == 1 {
        &parts[..]
    } else {
        return invalid_housing_import_path(
            "Import path must be a file in housing-exports or start with housing-exports/.",
        );
    };

    if relative_parts.is_empty() {
        return invalid_housing_import_path(
            "Import path must point to a file inside housing-exports.",
        );
    }

    let normalized = format!("{HOUSING_EXPORTS_DIR}/{}", relative_parts.join("/"));
    if fixed_string_serialized_len(&normalized) <= HOUSING_ADMIN_IMPORT_PATH_MAX_BYTES {
        Ok(normalized)
    } else {
        invalid_housing_import_path("Import path exceeds the 260-byte housing IPC limit.")
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::BinWrite;

    use crate::common::test_opcodes;
    use crate::packet::{PredefinedOpcode, ReadWriteIpcSegment};

    use super::*;

    fn serialize_custom_ipc(data: CustomIpcData) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        CustomIpcSegment::new(data)
            .write_le(&mut cursor)
            .expect("custom IPC segment should serialize");
        cursor.into_inner()
    }

    // Ensure that the IPC data size as reported matches up with what we write
    #[test]
    fn custom_ipc_sizes() {
        test_opcodes::<CustomIpcSegment>();
        assert_eq!(
            CustomIpcType::HousingSummaryResponse.calc_size(),
            HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES as u32
        );
        assert_eq!(
            CustomIpcType::HousingEstateDetailResponse.calc_size(),
            HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES as u32
        );
        assert_eq!(CustomIpcType::UpdateHousingEstateText.calc_size(), 220);
        assert_eq!(CustomIpcType::HousingEstateExported.calc_size(), 772);
        assert_eq!(
            CustomIpcType::ImportHousingEstate.calc_size(),
            HOUSING_ADMIN_IMPORT_PATH_MAX_BYTES as u32
        );
        assert_eq!(
            CustomIpcType::HousingEstateImportResult.calc_size(),
            HOUSING_ADMIN_MESSAGE_MAX_BYTES as u32
        );
    }

    #[test]
    fn custom_ipc_sizes_housing_summary_overflow_returns_valid_json_and_serializes() {
        let oversized = format!(
            r#"{{"rows":"{}"}}"#,
            "x".repeat(HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES)
        );
        let bounded = clamp_housing_summary_json_for_ipc(&oversized);

        assert!(bounded.len() <= HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES);
        let parsed: serde_json::Value =
            serde_json::from_str(&bounded).expect("bounded summary should stay valid JSON");
        assert_eq!(parsed["truncated"], true);

        let bytes = serialize_custom_ipc(CustomIpcData::HousingSummaryResponse { json: bounded });
        assert_eq!(
            bytes.len(),
            CustomIpcSegment::new(CustomIpcData::HousingSummaryResponse {
                json: String::new(),
            })
            .calc_size() as usize
        );
    }

    #[test]
    fn custom_ipc_sizes_housing_detail_overflow_returns_valid_json_and_serializes() {
        let oversized = format!(
            r#"{{"detail":"{}"}}"#,
            "가".repeat(HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES)
        );
        let bounded = clamp_housing_detail_json_for_ipc(&oversized);

        assert!(bounded.len() <= HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES);
        let parsed: serde_json::Value =
            serde_json::from_str(&bounded).expect("bounded detail should stay valid JSON");
        assert_eq!(parsed["truncated"], true);

        let bytes =
            serialize_custom_ipc(CustomIpcData::HousingEstateDetailResponse { json: bounded });
        assert_eq!(
            bytes.len(),
            CustomIpcSegment::new(CustomIpcData::HousingEstateDetailResponse {
                json: String::new(),
            })
            .calc_size() as usize
        );
    }

    #[test]
    fn custom_ipc_sizes_housing_text_fields_clamp_on_utf8_boundary() {
        let name = clamp_housing_admin_name_for_ipc("abcdefghijklmnopqrstu한글");
        let greeting = clamp_housing_admin_greeting_for_ipc(&format!("{}끝", "나".repeat(192)));

        assert!(name.len() <= HOUSING_ADMIN_NAME_MAX_BYTES);
        assert!(greeting.len() <= HOUSING_ADMIN_GREETING_MAX_BYTES);
        assert!(std::str::from_utf8(name.as_bytes()).is_ok());
        assert!(std::str::from_utf8(greeting.as_bytes()).is_ok());

        let bytes = serialize_custom_ipc(CustomIpcData::UpdateHousingEstateText {
            land_ident: 55,
            name,
            greeting,
        });
        assert_eq!(
            bytes.len(),
            CustomIpcSegment::new(CustomIpcData::UpdateHousingEstateText {
                land_ident: 55,
                name: String::new(),
                greeting: String::new(),
            })
            .calc_size() as usize
        );
    }

    #[test]
    fn custom_ipc_sizes_housing_export_fields_clamp_and_serialize() {
        let path = clamp_housing_export_path_for_ipc(&format!(r"C:\{}", "경".repeat(200)));
        let message =
            clamp_housing_message_for_ipc(&format!("{}{}", "m".repeat(600), "끝".repeat(32)));

        assert!(path.len() <= HOUSING_ADMIN_EXPORT_PATH_MAX_BYTES);
        assert!(message.len() <= HOUSING_ADMIN_MESSAGE_MAX_BYTES);
        assert!(std::str::from_utf8(path.as_bytes()).is_ok());
        assert!(std::str::from_utf8(message.as_bytes()).is_ok());

        let bytes = serialize_custom_ipc(CustomIpcData::HousingEstateExported { path, message });
        assert_eq!(
            bytes.len(),
            CustomIpcSegment::new(CustomIpcData::HousingEstateExported {
                path: String::new(),
                message: String::new(),
            })
            .calc_size() as usize
        );
    }

    #[test]
    fn housing_import_path_accepts_allowed_export_paths() {
        assert_eq!(
            validate_housing_import_path_for_ipc("estate-123.json")
                .expect("bare export file should be normalized"),
            "housing-exports/estate-123.json"
        );
        assert_eq!(
            validate_housing_import_path_for_ipc("housing-exports/estate-123.json")
                .expect("prefixed export file should be accepted"),
            "housing-exports/estate-123.json"
        );
    }

    #[test]
    fn housing_import_path_rejects_unsafe_paths() {
        for path in [
            "/tmp/estate-123.json",
            r"\temp\estate-123.json",
            "../estate-123.json",
            "housing-exports/../estate-123.json",
            "foo/estate-123.json",
            r"C:\temp\estate-123.json",
        ] {
            let error = validate_housing_import_path_for_ipc(path)
                .expect_err("unsafe import path should be rejected");

            assert!(error.len() <= HOUSING_ADMIN_MESSAGE_MAX_BYTES);
            assert!(std::str::from_utf8(error.as_bytes()).is_ok());
        }
    }

    #[test]
    fn custom_ipc_sizes_housing_import_path_rejects_oversized_input() {
        let path = format!("housing-exports/{}", "한".repeat(200));
        let error = validate_housing_import_path_for_ipc(&path)
            .expect_err("oversized import path should be rejected");

        assert!(error.len() <= HOUSING_ADMIN_MESSAGE_MAX_BYTES);
        assert!(std::str::from_utf8(error.as_bytes()).is_ok());
    }
}
