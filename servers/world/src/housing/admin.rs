use kawari::ipc::{
    kawari::{
        clamp_housing_admin_greeting_for_ipc, clamp_housing_admin_name_for_ipc,
        clamp_housing_detail_json_for_ipc, clamp_housing_summary_json_for_ipc,
    },
    zone::PlotSize,
};
use serde::{Deserialize, Serialize};

use crate::{
    database::{
        HousingEstate, HousingEstateDetailQuery, HousingEstateSummaryQueryRow,
        HousingFurnitureCounts,
    },
    housing::container::housing_container_kind,
};

const HOUSING_DETAIL_EXPORT_GUIDANCE: &str = "Housing detail exceeded the admin IPC payload limit. Use Export JSON for the full estate payload.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HousingAdminEstateSummaryRow {
    pub land_ident: i64,
    pub house_id: i64,
    pub owner_content_id: Option<i64>,
    pub owner_name: String,
    pub plot: String,
    pub kind: String,
    pub size: String,
    pub flags: i32,
    pub furniture_counts: HousingFurnitureCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HousingAdminFurnitureRow {
    pub land_ident: i64,
    pub container_type: i32,
    pub container_kind: String,
    pub slot: i32,
    pub item_id: i64,
    pub catalog_id: i32,
    pub stain: i32,
    pub placed: bool,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
    pub created_by_content_id: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HousingEstateAdminDetail {
    pub estate: HousingEstate,
    pub furniture_counts: HousingFurnitureCounts,
    pub furniture: Vec<HousingAdminFurnitureRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HousingAdminSelectedEstate {
    land_ident: i64,
    house_id: i64,
    owner_content_id: Option<i64>,
    owner_name: String,
    estate_name: String,
    greeting: String,
}

pub fn summary_ipc_json(rows: &[HousingEstateSummaryQueryRow]) -> String {
    let rows = admin_summary_rows(rows);
    bounded_summary_json(&rows)
}

fn admin_summary_rows(rows: &[HousingEstateSummaryQueryRow]) -> Vec<HousingAdminEstateSummaryRow> {
    rows.iter().map(admin_summary_row).collect()
}

fn admin_summary_row(row: &HousingEstateSummaryQueryRow) -> HousingAdminEstateSummaryRow {
    HousingAdminEstateSummaryRow {
        land_ident: row.estate.land_ident,
        house_id: row.estate.house_id,
        owner_content_id: row.estate.owner_content_id,
        owner_name: row.estate.owner_name.clone(),
        plot: housing_plot_label(&row.estate),
        kind: housing_estate_kind(&row.estate).to_string(),
        size: housing_estate_size(&row.estate).to_string(),
        flags: row.estate.flags,
        furniture_counts: row.furniture_counts.clone(),
    }
}

fn summary_json(rows: &[HousingAdminEstateSummaryRow]) -> String {
    serde_json::to_string(rows).unwrap_or_else(|_| "[]".to_string())
}

fn bounded_summary_json(rows: &[HousingAdminEstateSummaryRow]) -> String {
    let full_json = summary_json(rows);
    if clamp_housing_summary_json_for_ipc(&full_json) == full_json {
        return full_json;
    }

    let mut returned_rows = Vec::new();
    let mut bounded_json = summary_overflow_json(rows.len(), &returned_rows);
    for row in rows {
        let mut candidate_rows = returned_rows.clone();
        candidate_rows.push(row.clone());
        let candidate = summary_overflow_json(rows.len(), &candidate_rows);
        if clamp_housing_summary_json_for_ipc(&candidate) != candidate {
            break;
        }
        returned_rows = candidate_rows;
        bounded_json = candidate;
    }

    bounded_json
}

fn summary_overflow_json(total: usize, rows: &[HousingAdminEstateSummaryRow]) -> String {
    serde_json::to_string(&serde_json::json!({
        "estates": rows,
        "truncated": true,
        "total": total,
        "returned": rows.len(),
        "omitted": total.saturating_sub(rows.len()),
        "message": "Housing summary was truncated to fit the admin IPC payload limit.",
    }))
    .unwrap_or_else(|_| r#"{"error":"housing_summary_backend_failed"}"#.to_string())
}

pub fn detail_from_query(query: HousingEstateDetailQuery) -> HousingEstateAdminDetail {
    let furniture = query
        .furniture
        .into_iter()
        .map(|row| HousingAdminFurnitureRow {
            land_ident: row.land_ident,
            container_type: row.container_type,
            container_kind: housing_container_kind(row.container_type).to_string(),
            slot: row.slot,
            item_id: row.item_id,
            catalog_id: row.catalog_id,
            stain: row.stain,
            placed: row.placed,
            pos_x: row.pos_x,
            pos_y: row.pos_y,
            pos_z: row.pos_z,
            rotation: row.rotation,
            created_by_content_id: row.created_by_content_id,
            updated_at: row.updated_at,
        })
        .collect();

    HousingEstateAdminDetail {
        estate: query.estate,
        furniture_counts: query.furniture_counts,
        furniture,
    }
}

pub fn detail_json(detail: &HousingEstateAdminDetail) -> Option<String> {
    serde_json::to_string(detail).ok()
}

pub fn detail_ipc_json(detail: &HousingEstateAdminDetail) -> Result<String, serde_json::Error> {
    let full_json = serde_json::to_string(detail)?;

    if clamp_housing_detail_json_for_ipc(&full_json) == full_json {
        return Ok(full_json);
    }

    serde_json::to_string(&serde_json::json!({
        "error": "housing_detail_ipc_overflow",
        "truncated": true,
        "estate": selected_estate(&detail.estate),
        "land_ident": detail.estate.land_ident,
        "house_id": detail.estate.house_id,
        "furniture_counts": detail.furniture_counts,
        "furniture_omitted": detail.furniture.len(),
        "message": HOUSING_DETAIL_EXPORT_GUIDANCE,
    }))
}

fn selected_estate(estate: &HousingEstate) -> HousingAdminSelectedEstate {
    HousingAdminSelectedEstate {
        land_ident: estate.land_ident,
        house_id: estate.house_id,
        owner_content_id: estate.owner_content_id,
        owner_name: estate.owner_name.clone(),
        estate_name: clamp_housing_admin_name_for_ipc(&estate.estate_name),
        greeting: clamp_housing_admin_greeting_for_ipc(&estate.greeting),
    }
}

fn housing_estate_kind(estate: &HousingEstate) -> &'static str {
    if estate.is_apartment {
        "apartment"
    } else if estate.flags & 0x10 != 0 {
        "free_company_estate"
    } else {
        "personal_estate"
    }
}

fn housing_estate_size(estate: &HousingEstate) -> &'static str {
    if estate.is_apartment {
        "apartment"
    } else {
        match PlotSize::from_repr(estate.plot_size as u8) {
            Some(PlotSize::Small) => "small",
            Some(PlotSize::Medium) => "medium",
            Some(PlotSize::Large) => "large",
            _ => "unknown",
        }
    }
}

fn housing_plot_label(estate: &HousingEstate) -> String {
    if estate.is_apartment {
        format!(
            "Ward {} Apartment {}",
            estate.ward_index + 1,
            estate.room_number
        )
    } else {
        let subdivision = if estate.division != 0 {
            " Subdivision"
        } else {
            ""
        };
        format!(
            "Ward {}{} Plot {}",
            estate.ward_index + 1,
            subdivision,
            estate.plot_index + 1
        )
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::BinWrite;
    use kawari::ipc::kawari::{
        CustomIpcData, CustomIpcSegment, HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES,
        HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES,
    };
    use kawari::packet::ReadWriteIpcSegment;

    use crate::database::HousingEstate;

    use super::*;

    fn estate() -> HousingEstate {
        HousingEstate {
            land_ident: 42,
            house_id: 9001,
            owner_content_id: Some(100),
            owner_name: "Tester".to_string(),
            estate_name: "Serialization Estate".to_string(),
            greeting: "Welcome to the fixture.".to_string(),
            exterior_json: "{}".to_string(),
            interior_json: "{}".to_string(),
            light_level: 3,
            flags: 0x10,
            ..Default::default()
        }
    }

    fn serialize_custom_ipc(data: CustomIpcData) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        CustomIpcSegment::new(data)
            .write_le(&mut cursor)
            .expect("custom IPC segment should serialize");
        cursor.into_inner()
    }

    #[test]
    fn housing_admin_summary_json_serializes_compact_rows() {
        let rows = (0..11)
            .map(|idx| HousingAdminEstateSummaryRow {
                land_ident: 42 + idx,
                house_id: 9001 + idx,
                owner_content_id: Some(100 + idx),
                owner_name: format!("Tester-{idx:02}"),
                plot: format!("Ward 1 Plot {}", idx + 1),
                kind: "free_company_estate".to_string(),
                size: "large".to_string(),
                flags: 0x10,
                furniture_counts: HousingFurnitureCounts {
                    indoor_placed: 1,
                    indoor_storeroom: 2,
                    outdoor_placed: 3,
                    outdoor_storeroom: 4,
                    total: 10,
                },
            })
            .collect::<Vec<_>>();

        let json = summary_json(&rows);
        assert!(json.len() <= HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES);

        let summary: serde_json::Value =
            serde_json::from_str(&json).expect("summary JSON should parse");
        let row = &summary
            .as_array()
            .expect("summary should serialize to an array")[0];

        assert_eq!(row["land_ident"].as_i64(), Some(42));
        assert_eq!(row["owner_name"].as_str(), Some("Tester-00"));
        assert_eq!(row["kind"].as_str(), Some("free_company_estate"));
        assert_eq!(row["furniture_counts"]["total"].as_u64(), Some(10));
        assert!(row.get("estate_name").is_none());
        assert!(row.get("greeting").is_none());

        let bytes = serialize_custom_ipc(CustomIpcData::HousingSummaryResponse { json });
        assert_eq!(
            bytes.len(),
            CustomIpcSegment::new(CustomIpcData::HousingSummaryResponse {
                json: String::new(),
            })
            .calc_size() as usize
        );
    }

    #[test]
    fn housing_admin_summary_ipc_json_returns_partial_rows_when_full_payload_overflows() {
        let rows = (0..200)
            .map(|idx| HousingAdminEstateSummaryRow {
                land_ident: 42 + idx,
                house_id: 9001 + idx,
                owner_content_id: Some(100 + idx),
                owner_name: format!("Tester-{idx:03}"),
                plot: format!("Ward 1 Plot {}", idx + 1),
                kind: "free_company_estate".to_string(),
                size: "large".to_string(),
                flags: 0x10,
                furniture_counts: HousingFurnitureCounts {
                    indoor_placed: 1,
                    indoor_storeroom: 2,
                    outdoor_placed: 3,
                    outdoor_storeroom: 4,
                    total: 10,
                },
            })
            .collect::<Vec<_>>();

        let json = bounded_summary_json(&rows);
        assert!(json.len() <= HOUSING_ADMIN_SUMMARY_JSON_MAX_BYTES);

        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("summary fallback JSON should parse");
        assert_eq!(parsed["truncated"], true);
        assert!(
            parsed["omitted"]
                .as_u64()
                .is_some_and(|omitted| omitted > 0)
        );
        assert!(
            parsed["estates"]
                .as_array()
                .is_some_and(|estates| !estates.is_empty())
        );
    }

    #[test]
    fn housing_admin_detail_json_serializes_counts_and_furniture_rows() {
        let detail = HousingEstateAdminDetail {
            estate: estate(),
            furniture_counts: HousingFurnitureCounts {
                indoor_placed: 1,
                indoor_storeroom: 0,
                outdoor_placed: 0,
                outdoor_storeroom: 1,
                total: 2,
            },
            furniture: vec![
                HousingAdminFurnitureRow {
                    land_ident: 42,
                    container_type: 25003,
                    container_kind: "indoor_placed".to_string(),
                    slot: 0,
                    item_id: 2000,
                    catalog_id: 88,
                    stain: 5,
                    placed: true,
                    pos_x: 1.0,
                    pos_y: 2.0,
                    pos_z: 3.0,
                    rotation: 0.5,
                    created_by_content_id: Some(100),
                    updated_at: 123,
                },
                HousingAdminFurnitureRow {
                    land_ident: 42,
                    container_type: 27000,
                    container_kind: "outdoor_storeroom".to_string(),
                    slot: 1,
                    item_id: 2001,
                    catalog_id: 89,
                    stain: 0,
                    placed: false,
                    pos_x: 0.0,
                    pos_y: 0.0,
                    pos_z: 0.0,
                    rotation: 0.0,
                    created_by_content_id: None,
                    updated_at: 124,
                },
            ],
        };

        let json = detail_json(&detail).expect("detail should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("detail JSON should parse");
        let furniture = parsed["furniture"]
            .as_array()
            .expect("detail should include furniture rows");

        assert_eq!(parsed["estate"]["land_ident"].as_i64(), Some(42));
        assert_eq!(parsed["estate"]["owner_name"].as_str(), Some("Tester"));
        assert_eq!(
            parsed["furniture_counts"]["indoor_placed"].as_u64(),
            Some(1)
        );
        assert_eq!(
            parsed["furniture_counts"]["outdoor_storeroom"].as_u64(),
            Some(1)
        );
        assert_eq!(furniture.len(), 2);
        assert_eq!(
            furniture[0]["container_kind"].as_str(),
            Some("indoor_placed")
        );
        assert_eq!(
            furniture[1]["container_kind"].as_str(),
            Some("outdoor_storeroom")
        );
    }

    #[test]
    fn housing_admin_detail_ipc_json_uses_bounded_fallback() {
        let mut estate = estate();
        estate.exterior_json = "e".repeat(2048);
        estate.interior_json = "i".repeat(2048);

        let detail = HousingEstateAdminDetail {
            estate,
            furniture_counts: HousingFurnitureCounts {
                indoor_placed: 180,
                total: 180,
                ..Default::default()
            },
            furniture: (0..180)
                .map(|slot| HousingAdminFurnitureRow {
                    land_ident: 42,
                    container_type: 25003,
                    container_kind: "indoor_placed".to_string(),
                    slot,
                    item_id: 3_000 + slot as i64,
                    catalog_id: 400 + slot,
                    stain: slot % 8,
                    placed: true,
                    pos_x: slot as f32,
                    pos_y: slot as f32 / 2.0,
                    pos_z: slot as f32 / 3.0,
                    rotation: slot as f32 / 10.0,
                    created_by_content_id: Some(100),
                    updated_at: 123,
                })
                .collect(),
        };

        let bounded = detail_ipc_json(&detail).expect("bounded detail should serialize");
        assert!(bounded.len() <= HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES);

        let parsed: serde_json::Value =
            serde_json::from_str(&bounded).expect("bounded detail should parse");
        assert_eq!(
            parsed["error"].as_str(),
            Some("housing_detail_ipc_overflow")
        );
        assert_eq!(parsed["truncated"], true);
        assert_eq!(parsed["land_ident"].as_i64(), Some(42));
        assert_eq!(parsed["house_id"].as_i64(), Some(9001));
        assert_eq!(
            parsed["furniture_counts"]["indoor_placed"].as_u64(),
            Some(180)
        );
        assert_eq!(parsed["furniture_omitted"].as_u64(), Some(180));
        assert!(
            parsed["message"]
                .as_str()
                .is_some_and(|message| message.contains("Export JSON"))
        );

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
    fn housing_admin_detail_ipc_json_fallback_omits_verbose_estate_json() {
        let mut estate = estate();
        estate.exterior_json = "e".repeat(HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES);
        estate.interior_json = "i".repeat(HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES);

        let detail = HousingEstateAdminDetail {
            estate,
            furniture_counts: HousingFurnitureCounts::default(),
            furniture: Vec::new(),
        };

        let bounded = detail_ipc_json(&detail).expect("bounded detail should serialize");
        assert!(bounded.len() <= HOUSING_ADMIN_DETAIL_JSON_MAX_BYTES);

        let parsed: serde_json::Value =
            serde_json::from_str(&bounded).expect("bounded fallback should parse");
        assert_eq!(
            parsed["error"].as_str(),
            Some("housing_detail_ipc_overflow")
        );
        assert_eq!(parsed["estate"]["land_ident"].as_i64(), Some(42));
        assert!(parsed["estate"].get("exterior_json").is_none());
        assert!(parsed["estate"].get("interior_json").is_none());
    }
}
