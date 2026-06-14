use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    HousingFurniture,
    inventory::{
        HOUSING_INTERIOR_PLACED_PAGE_COUNT, MAX_HOUSING_INTERIOR_STORAGE, MAX_LARGE_STORAGE,
        indoor_container_for_flat_slot,
    },
    lua::{HousingInteriorField, HousingPresetScope},
};
use kawari::{common::ContainerType, ipc::zone::PlotSize};

const DEFAULT_REMAKE_PLACE_LAYOUT_ROOT: &str = r"E:\FF14\DT Mods\HousingLayout\Large";
const REMAKE_PLACE_LAYOUT_ROOT_ENV: &str = "KAWARI_REMAKE_PLACE_LAYOUT_ROOT";
const INTERIOR_PLACED_CAPACITY: usize =
    MAX_HOUSING_INTERIOR_STORAGE * HOUSING_INTERIOR_PLACED_PAGE_COUNT;
const EXTERIOR_PLACED_CAPACITY: usize = MAX_LARGE_STORAGE;
pub(super) const REMAKE_PLACE_ITEM_UI_CATEGORY_INTERIOR_WALL: u8 = 73;
pub(super) const REMAKE_PLACE_ITEM_UI_CATEGORY_FLOORING: u8 = 74;
pub(super) const REMAKE_PLACE_ITEM_UI_CATEGORY_CEILING_LIGHT: u8 = 75;

#[derive(Debug, Clone)]
pub(super) struct RemakePlaceImportRows {
    pub rows: Vec<HousingFurniture>,
    pub indoor_imported: usize,
    pub outdoor_imported: usize,
    pub skipped_missing_item_id: usize,
    pub skipped_missing_catalog: usize,
    pub skipped_capacity: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RemakePlaceInteriorFixtureUpdates {
    pub renovation_row_id: Option<u16>,
    pub fixture_updates: Vec<(HousingInteriorField, u32)>,
    pub skipped_missing_item_id: usize,
    pub skipped_missing_item_data: usize,
    pub skipped_wrong_category: usize,
}

#[derive(Debug, Clone)]
pub(super) struct RemakePlacePresetPath {
    pub root: PathBuf,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RemakePlaceLayout {
    #[serde(default = "default_layout_scale")]
    interior_scale: f32,
    #[serde(default)]
    interior_fixture: Vec<RemakePlaceInteriorFixture>,
    #[serde(default)]
    interior_furniture: Vec<RemakePlaceFurniture>,
    #[serde(default = "default_layout_scale")]
    exterior_scale: f32,
    #[serde(default)]
    exterior_furniture: Vec<RemakePlaceFurniture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemakePlaceInteriorFixture {
    #[serde(default)]
    level: String,
    #[serde(default, rename = "type")]
    fixture_type: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    item_id: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemakePlaceFurniture {
    #[serde(default)]
    item_id: u32,
    #[serde(default)]
    transform: RemakePlaceTransform,
    #[serde(default)]
    properties: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    attachments: Vec<RemakePlaceFurniture>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemakePlaceTransform {
    #[serde(default = "default_location")]
    location: [f32; 3],
    #[serde(default = "default_rotation")]
    rotation: [f32; 4],
}

impl Default for RemakePlaceTransform {
    fn default() -> Self {
        Self {
            location: default_location(),
            rotation: default_rotation(),
        }
    }
}

pub(super) fn remake_place_layout_root() -> PathBuf {
    std::env::var_os(REMAKE_PLACE_LAYOUT_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_REMAKE_PLACE_LAYOUT_ROOT))
}

pub(super) fn resolve_remake_place_preset_path(
    input: &str,
) -> Result<RemakePlacePresetPath, String> {
    resolve_remake_place_preset_path_under_root(input, &remake_place_layout_root())
}

fn resolve_remake_place_preset_path_under_root(
    input: &str,
    root: &Path,
) -> Result<RemakePlacePresetPath, String> {
    let input = input.trim().trim_matches('"');
    if input.is_empty() {
        return Err("Preset path is empty.".to_string());
    }

    let candidate = PathBuf::from(input);
    if candidate.is_absolute() {
        let path = canonical_json_file(&candidate)?;
        let root = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(PathBuf::new);
        return Ok(RemakePlacePresetPath { root, path });
    }

    let root = root
        .canonicalize()
        .map_err(|error| format!("Unable to open ReMakePlace layout root {root:?}: {error}"))?;

    let direct_candidate = root.join(&candidate);

    if direct_candidate.is_file() {
        let path = canonical_json_file_under_root(&root, &direct_candidate)?;
        return Ok(RemakePlacePresetPath { root, path });
    }

    let found = find_remake_place_preset_by_name(&root, input)?
        .ok_or_else(|| format!("ReMakePlace preset not found under {root:?}: {input}"))?;
    Ok(RemakePlacePresetPath { root, path: found })
}

pub(super) fn parse_remake_place_layout_file(path: &Path) -> Result<RemakePlaceLayout, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Unable to read ReMakePlace preset {path:?}: {error}"))?;
    let json = decode_text_file_bytes(&bytes)
        .map_err(|error| format!("Unable to decode ReMakePlace preset {path:?}: {error}"))?;
    parse_remake_place_layout_json(&json)
        .map_err(|error| format!("Unable to parse ReMakePlace preset {path:?}: {error}"))
}

pub(super) fn parse_remake_place_layout_json(
    json: &str,
) -> Result<RemakePlaceLayout, serde_json::Error> {
    serde_json::from_str(json)
}

pub(super) fn build_remake_place_furniture_rows<F, S>(
    layout: &RemakePlaceLayout,
    land_ident: i64,
    created_by_content_id: Option<i64>,
    mut catalog_id_for_item: F,
    mut stain_for_rgb: S,
    scope: HousingPresetScope,
) -> RemakePlaceImportRows
where
    F: FnMut(u32) -> Option<u16>,
    S: FnMut([u8; 3]) -> Option<u8>,
{
    let mut result = RemakePlaceImportRows {
        rows: Vec::new(),
        indoor_imported: 0,
        outdoor_imported: 0,
        skipped_missing_item_id: 0,
        skipped_missing_catalog: 0,
        skipped_capacity: 0,
    };

    if scope.includes_interior() {
        let mut flat_slot = 0;
        for furniture in flattened_furniture(&layout.interior_furniture) {
            let Some(row) = build_row(
                furniture,
                safe_layout_scale(layout.interior_scale),
                land_ident,
                created_by_content_id,
                &mut catalog_id_for_item,
                &mut stain_for_rgb,
                true,
                &mut flat_slot,
                &mut result,
            ) else {
                continue;
            };
            result.rows.push(row);
            result.indoor_imported += 1;
        }
    }

    if scope.includes_exterior() {
        let mut flat_slot = 0;
        for furniture in flattened_furniture(&layout.exterior_furniture) {
            let Some(row) = build_row(
                furniture,
                safe_layout_scale(layout.exterior_scale),
                land_ident,
                created_by_content_id,
                &mut catalog_id_for_item,
                &mut stain_for_rgb,
                false,
                &mut flat_slot,
                &mut result,
            ) else {
                continue;
            };
            result.rows.push(row);
            result.outdoor_imported += 1;
        }
    }

    result
}

pub(super) fn build_remake_place_interior_fixture_updates<F>(
    layout: &RemakePlaceLayout,
    plot_size: PlotSize,
    mut item_data_for_item: F,
) -> RemakePlaceInteriorFixtureUpdates
where
    F: FnMut(u32) -> Option<(u32, u8)>,
{
    let mut result = RemakePlaceInteriorFixtureUpdates::default();

    for fixture in &layout.interior_fixture {
        let fixture_type = normalize_remake_place_fixture_token(&fixture.fixture_type);
        if fixture_type == "district" {
            if let Some(row_id) = remake_place_renovation_row_id(&fixture.name, plot_size) {
                result.renovation_row_id = Some(row_id);
            }
            continue;
        }

        let Some(field) =
            remake_place_interior_fixture_field(&fixture.level, &fixture.fixture_type)
        else {
            continue;
        };

        if fixture.item_id == 0 {
            result.skipped_missing_item_id += 1;
            continue;
        }

        let Some((additional_data, item_ui_category)) = item_data_for_item(fixture.item_id) else {
            result.skipped_missing_item_data += 1;
            continue;
        };

        if remake_place_expected_item_ui_category(field) != Some(item_ui_category) {
            result.skipped_wrong_category += 1;
            continue;
        }

        upsert_fixture_update(&mut result.fixture_updates, field, additional_data);
    }

    result
}

fn build_row<F, S>(
    furniture: &RemakePlaceFurniture,
    layout_scale: f32,
    land_ident: i64,
    created_by_content_id: Option<i64>,
    catalog_id_for_item: &mut F,
    stain_for_rgb: &mut S,
    indoors: bool,
    flat_slot: &mut usize,
    result: &mut RemakePlaceImportRows,
) -> Option<HousingFurniture>
where
    F: FnMut(u32) -> Option<u16>,
    S: FnMut([u8; 3]) -> Option<u8>,
{
    if furniture.item_id == 0 {
        result.skipped_missing_item_id += 1;
        return None;
    }

    let Some(catalog_id) = catalog_id_for_item(furniture.item_id) else {
        result.skipped_missing_catalog += 1;
        return None;
    };

    let capacity = if indoors {
        INTERIOR_PLACED_CAPACITY
    } else {
        EXTERIOR_PLACED_CAPACITY
    };
    if *flat_slot >= capacity {
        result.skipped_capacity += 1;
        return None;
    }

    let (container_type, slot) = if indoors {
        indoor_container_for_flat_slot(*flat_slot as u16)
    } else {
        Some((ContainerType::HousingExteriorPlacedItems, *flat_slot as u16))
    }?;
    *flat_slot += 1;

    let location = furniture.transform.location;
    let rotation = furniture.transform.rotation;
    let stain = remake_place_rgb_from_properties(&furniture.properties)
        .and_then(stain_for_rgb)
        .unwrap_or(0);

    Some(HousingFurniture {
        land_ident,
        container_type: container_type_to_i32(container_type),
        slot: slot as i32,
        item_id: furniture.item_id as i64,
        catalog_id: catalog_id as i32,
        stain: stain as i32,
        placed: true,
        pos_x: location[0] / layout_scale,
        pos_y: location[2] / layout_scale,
        pos_z: location[1] / layout_scale,
        rotation: -compute_z_angle(rotation),
        created_by_content_id,
        updated_at: 0,
    })
}

fn container_type_to_i32(container_type: ContainerType) -> i32 {
    container_type as u16 as i32
}

fn remake_place_interior_fixture_field(
    level: &str,
    fixture_type: &str,
) -> Option<HousingInteriorField> {
    let level = normalize_remake_place_fixture_token(level);
    let fixture_type = normalize_remake_place_fixture_token(fixture_type);

    match (level.as_str(), fixture_type.as_str()) {
        ("groundfloor" | "ground" | "firstfloor" | "1stfloor", "wall") => {
            Some(HousingInteriorField::GroundWalls)
        }
        ("groundfloor" | "ground" | "firstfloor" | "1stfloor", "floor") => {
            Some(HousingInteriorField::GroundFloor)
        }
        ("groundfloor" | "ground" | "firstfloor" | "1stfloor", "light" | "ceilinglight") => {
            Some(HousingInteriorField::GroundChandelier)
        }
        ("upperfloor" | "topfloor" | "secondfloor" | "2ndfloor", "wall") => {
            Some(HousingInteriorField::TopWalls)
        }
        ("upperfloor" | "topfloor" | "secondfloor" | "2ndfloor", "floor") => {
            Some(HousingInteriorField::TopFloor)
        }
        ("upperfloor" | "topfloor" | "secondfloor" | "2ndfloor", "light" | "ceilinglight") => {
            Some(HousingInteriorField::TopChandelier)
        }
        ("basement" | "cellar", "wall") => Some(HousingInteriorField::CellarWalls),
        ("basement" | "cellar", "floor") => Some(HousingInteriorField::CellarFloor),
        ("basement" | "cellar", "light" | "ceilinglight") => {
            Some(HousingInteriorField::CellarChandelier)
        }
        _ => None,
    }
}

fn remake_place_expected_item_ui_category(field: HousingInteriorField) -> Option<u8> {
    match field {
        HousingInteriorField::GroundWalls
        | HousingInteriorField::TopWalls
        | HousingInteriorField::CellarWalls => Some(REMAKE_PLACE_ITEM_UI_CATEGORY_INTERIOR_WALL),
        HousingInteriorField::GroundFloor
        | HousingInteriorField::TopFloor
        | HousingInteriorField::CellarFloor => Some(REMAKE_PLACE_ITEM_UI_CATEGORY_FLOORING),
        HousingInteriorField::GroundChandelier
        | HousingInteriorField::TopChandelier
        | HousingInteriorField::CellarChandelier => {
            Some(REMAKE_PLACE_ITEM_UI_CATEGORY_CEILING_LIGHT)
        }
        _ => None,
    }
}

fn remake_place_renovation_row_id(name: &str, plot_size: PlotSize) -> Option<u16> {
    let normalized = normalize_remake_place_fixture_token(name);
    let base = if normalized.contains("minimalistdark") || normalized.contains("simpledark") {
        19
    } else if normalized.contains("minimalist") || normalized.contains("simple") {
        16
    } else if normalized.contains("empyreum") {
        13
    } else if normalized.contains("shirogane") {
        10
    } else if normalized.contains("goblet") {
        7
    } else if normalized.contains("lavender") {
        4
    } else if normalized.contains("mist") {
        1
    } else {
        return None;
    };

    Some(base + plot_size_index(plot_size))
}

fn plot_size_index(plot_size: PlotSize) -> u16 {
    match plot_size {
        PlotSize::Small => 0,
        PlotSize::Medium => 1,
        PlotSize::Large => 2,
    }
}

fn upsert_fixture_update(
    updates: &mut Vec<(HousingInteriorField, u32)>,
    field: HousingInteriorField,
    value: u32,
) {
    if let Some((_, existing)) = updates
        .iter_mut()
        .find(|(existing_field, _)| *existing_field == field)
    {
        *existing = value;
        return;
    }

    updates.push((field, value));
}

fn flattened_furniture(furniture: &[RemakePlaceFurniture]) -> Vec<&RemakePlaceFurniture> {
    let mut flattened = Vec::new();
    for item in furniture {
        flatten_furniture(item, &mut flattened);
    }
    flattened
}

fn flatten_furniture<'a>(
    furniture: &'a RemakePlaceFurniture,
    flattened: &mut Vec<&'a RemakePlaceFurniture>,
) {
    flattened.push(furniture);
    for attachment in &furniture.attachments {
        flatten_furniture(attachment, flattened);
    }
}

fn canonical_json_file(path: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("Unable to open ReMakePlace preset {path:?}: {error}"))?;

    if path.extension().and_then(OsStr::to_str) != Some("json") {
        return Err(format!("ReMakePlace preset is not a .json file: {path:?}"));
    }

    Ok(path)
}

fn canonical_json_file_under_root(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = canonical_json_file(path)?;

    if !path.starts_with(root) {
        return Err(format!(
            "ReMakePlace preset must be under configured layout root {root:?}: {path:?}"
        ));
    }

    Ok(path)
}

fn decode_text_file_bytes(bytes: &[u8]) -> Result<String, String> {
    if let Some(bytes) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string());
    }

    if let Some(bytes) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16_bytes(bytes, u16::from_le_bytes);
    }

    if let Some(bytes) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16_bytes(bytes, u16::from_be_bytes);
    }

    String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string())
}

fn decode_utf16_bytes(bytes: &[u8], convert: impl Fn([u8; 2]) -> u16) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("UTF-16 data has an odd byte length.".to_string());
    }

    let words = bytes
        .chunks_exact(2)
        .map(|chunk| convert([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&words).map_err(|error| error.to_string())
}

fn remake_place_rgb_from_properties(
    properties: &serde_json::Map<String, serde_json::Value>,
) -> Option<[u8; 3]> {
    let color = properties.get("color")?.as_str()?;
    parse_remake_place_rgb(color)
}

fn parse_remake_place_rgb(color: &str) -> Option<[u8; 3]> {
    let hex = color.as_bytes().get(..6)?;
    if !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let hex = std::str::from_utf8(hex).ok()?;

    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

fn find_remake_place_preset_by_name(root: &Path, query: &str) -> Result<Option<PathBuf>, String> {
    let query = normalize_layout_name(query);
    let mut stack = vec![root.to_path_buf()];
    let mut matches = Vec::new();

    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .map_err(|error| {
                format!("Unable to scan ReMakePlace layout directory {dir:?}: {error}")
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                format!("Unable to scan ReMakePlace layout directory {dir:?}: {error}")
            })?;
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                format!("Unable to inspect ReMakePlace layout path {path:?}: {error}")
            })?;

            if file_type.is_dir() {
                stack.push(path);
                continue;
            }

            if path.extension().and_then(OsStr::to_str) != Some("json") {
                continue;
            }

            let file_stem = path
                .file_stem()
                .and_then(OsStr::to_str)
                .map(normalize_layout_name);
            let parent_name = path
                .parent()
                .and_then(Path::file_name)
                .and_then(OsStr::to_str)
                .map(normalize_layout_name);
            let file_name = path
                .file_name()
                .and_then(OsStr::to_str)
                .map(normalize_layout_name);

            if file_stem.as_deref() == Some(&query)
                || parent_name.as_deref() == Some(&query)
                || file_name.as_deref() == Some(&query)
            {
                matches.push(canonical_json_file_under_root(root, &path)?);
            }
        }
    }

    matches.sort();
    matches.dedup();

    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(format!(
            "ReMakePlace preset name is ambiguous under {root:?}: {query}. Use a specific relative .json path. Matches: {}",
            matches
                .iter()
                .map(|path| path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .display()
                    .to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn normalize_layout_name(value: &str) -> String {
    let value = value.trim().trim_matches('"').to_lowercase();
    value
        .strip_suffix(".json")
        .unwrap_or(value.as_str())
        .to_string()
}

fn normalize_remake_place_fixture_token(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn compute_z_angle(rotation: [f32; 4]) -> f32 {
    let [x, y, z, w] = rotation;
    let siny_cosp = 2.0 * (w * z + x * y);
    let cosy_cosp = 1.0 - 2.0 * (y * y + z * z);
    siny_cosp.atan2(cosy_cosp)
}

fn safe_layout_scale(scale: f32) -> f32 {
    if scale.abs() < f32::EPSILON {
        1.0
    } else {
        scale
    }
}

fn default_layout_scale() -> f32 {
    1.0
}

fn default_location() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

fn default_rotation() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawari::common::ContainerType;

    #[test]
    fn builds_rows_from_remake_place_layout_using_plugin_transform_rules() {
        let json = r#"
        {
            "houseSize": "Large",
            "interiorScale": 100,
            "interiorFurniture": [
                {
                    "name": "Parent",
                    "itemId": 1001,
                    "transform": {
                        "location": [150.0, 250.0, -350.0],
                        "rotation": [0.0, 0.0, 0.70710677, 0.70710677],
                        "scale": [1.0, 1.0, 1.0]
                    },
                    "attachments": [
                        {
                            "name": "Child",
                            "itemId": 1002,
                            "transform": {
                                "location": [400.0, 500.0, 600.0],
                                "rotation": [0.0, 0.0, 0.0, 1.0],
                                "scale": [1.0, 1.0, 1.0]
                            }
                        }
                    ]
                }
            ],
            "exteriorScale": 1,
            "exteriorFurniture": [
                {
                    "name": "Yard",
                    "itemId": 2001,
                    "transform": {
                        "location": [1.0, 2.0, 3.0],
                        "rotation": [0.0, 0.0, -0.38268343, 0.9238795],
                        "scale": [1.0, 1.0, 1.0]
                    }
                }
            ]
        }
        "#;

        let layout = parse_remake_place_layout_json(json).expect("layout should parse");
        let result = build_remake_place_furniture_rows(
            &layout,
            1234,
            Some(5678),
            |item_id| Some((item_id - 1000) as u16),
            |_| None,
            HousingPresetScope::All,
        );

        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.indoor_imported, 2);
        assert_eq!(result.outdoor_imported, 1);
        assert_eq!(result.skipped_missing_catalog, 0);
        assert_eq!(result.skipped_capacity, 0);

        let first = &result.rows[0];
        assert_eq!(first.land_ident, 1234);
        assert_eq!(
            first.container_type,
            ContainerType::HousingInteriorPlacedItems1 as u16 as i32
        );
        assert_eq!(first.slot, 0);
        assert_eq!(first.item_id, 1001);
        assert_eq!(first.catalog_id, 1);
        assert_eq!(first.created_by_content_id, Some(5678));
        assert!((first.pos_x - 1.5).abs() < 0.0001);
        assert!((first.pos_y + 3.5).abs() < 0.0001);
        assert!((first.pos_z - 2.5).abs() < 0.0001);
        assert!((first.rotation + std::f32::consts::FRAC_PI_2).abs() < 0.0001);

        let child = &result.rows[1];
        assert_eq!(child.item_id, 1002);
        assert_eq!(child.slot, 1);
        assert!((child.pos_x - 4.0).abs() < 0.0001);
        assert!((child.pos_y - 6.0).abs() < 0.0001);
        assert!((child.pos_z - 5.0).abs() < 0.0001);
        assert!(child.rotation.abs() < 0.0001);

        let yard = &result.rows[2];
        assert_eq!(
            yard.container_type,
            ContainerType::HousingExteriorPlacedItems as u16 as i32
        );
        assert_eq!(yard.slot, 0);
        assert_eq!(yard.item_id, 2001);
        assert_eq!(yard.catalog_id, 1001);
        assert!((yard.pos_x - 1.0).abs() < 0.0001);
        assert!((yard.pos_y - 3.0).abs() < 0.0001);
        assert!((yard.pos_z - 2.0).abs() < 0.0001);
        assert!((yard.rotation - std::f32::consts::FRAC_PI_4).abs() < 0.0001);
    }

    #[test]
    fn imports_remake_place_color_properties_as_housing_stains() {
        let json = r#"
        {
            "interiorScale": 1,
            "interiorFurniture": [
                {
                    "itemId": 1001,
                    "properties": {
                        "color": "112233FF"
                    },
                    "attachments": [
                        {
                            "itemId": 1002,
                            "properties": {
                                "color": "445566"
                            }
                        }
                    ]
                },
                {
                    "itemId": 1003,
                    "properties": {
                        "color": "not-a-color"
                    }
                }
            ]
        }
        "#;

        let layout = parse_remake_place_layout_json(json).expect("layout should parse");
        let result = build_remake_place_furniture_rows(
            &layout,
            1234,
            Some(5678),
            |item_id| Some((item_id - 1000) as u16),
            |rgb| match rgb {
                [0x11, 0x22, 0x33] => Some(42),
                [0x44, 0x55, 0x66] => Some(77),
                _ => None,
            },
            HousingPresetScope::Interior,
        );

        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.rows[0].stain, 42);
        assert_eq!(result.rows[1].stain, 77);
        assert_eq!(result.rows[2].stain, 0);
    }

    #[test]
    fn imports_remake_place_interior_fixtures_and_style() {
        let json = r#"
        {
            "interiorFixture": [
                { "level": "", "type": "District", "name": "Minimalist", "itemId": 0 },
                { "level": "Basement", "type": "Floor", "name": "Basement Floor", "itemId": 10 },
                { "level": "Basement", "type": "Wall", "name": "Basement Wall", "itemId": 11 },
                { "level": "Ground Floor", "type": "Light", "name": "Ground Light", "itemId": 12 },
                { "level": "Upper Floor", "type": "Wall", "name": "Upper Wall", "itemId": 13 },
                { "level": "Ground Floor", "type": "Floor", "name": "Wrong Category", "itemId": 14 }
            ]
        }
        "#;

        let layout = parse_remake_place_layout_json(json).expect("layout should parse");
        let result =
            build_remake_place_interior_fixture_updates(&layout, PlotSize::Large, |item_id| {
                match item_id {
                    10 => Some((110, REMAKE_PLACE_ITEM_UI_CATEGORY_FLOORING)),
                    11 => Some((111, REMAKE_PLACE_ITEM_UI_CATEGORY_INTERIOR_WALL)),
                    12 => Some((112, REMAKE_PLACE_ITEM_UI_CATEGORY_CEILING_LIGHT)),
                    13 => Some((113, REMAKE_PLACE_ITEM_UI_CATEGORY_INTERIOR_WALL)),
                    14 => Some((114, REMAKE_PLACE_ITEM_UI_CATEGORY_INTERIOR_WALL)),
                    _ => None,
                }
            });

        assert_eq!(result.renovation_row_id, Some(18));
        assert_eq!(result.fixture_updates.len(), 4);
        assert!(
            result
                .fixture_updates
                .contains(&(HousingInteriorField::CellarFloor, 110,))
        );
        assert!(
            result
                .fixture_updates
                .contains(&(HousingInteriorField::CellarWalls, 111,))
        );
        assert!(
            result
                .fixture_updates
                .contains(&(HousingInteriorField::GroundChandelier, 112,))
        );
        assert!(
            result
                .fixture_updates
                .contains(&(HousingInteriorField::TopWalls, 113,))
        );
        assert_eq!(result.skipped_missing_item_id, 0);
        assert_eq!(result.skipped_missing_item_data, 0);
        assert_eq!(result.skipped_wrong_category, 1);
    }

    #[test]
    fn skips_missing_catalogs_and_rows_beyond_supported_slots() {
        let mut interior = String::new();
        for item_id in 1..=602 {
            if !interior.is_empty() {
                interior.push(',');
            }
            interior.push_str(&format!(
                r#"{{
                    "itemId": {item_id},
                    "transform": {{
                        "location": [0.0, 0.0, 0.0],
                        "rotation": [0.0, 0.0, 0.0, 1.0],
                        "scale": [1.0, 1.0, 1.0]
                    }}
                }}"#
            ));
        }

        let json = format!(
            r#"{{
                "interiorScale": 1,
                "interiorFurniture": [{interior}],
                "exteriorScale": 1,
                "exteriorFurniture": []
            }}"#
        );

        let layout = parse_remake_place_layout_json(&json).expect("layout should parse");
        let result = build_remake_place_furniture_rows(
            &layout,
            1234,
            None,
            |item_id| (item_id != 2).then_some(item_id as u16),
            |_| None,
            HousingPresetScope::All,
        );

        assert_eq!(result.rows.len(), 600);
        assert_eq!(result.indoor_imported, 600);
        assert_eq!(result.skipped_missing_catalog, 1);
        assert_eq!(result.skipped_capacity, 1);
        assert_eq!(
            result.rows.last().map(|row| row.container_type),
            Some(ContainerType::HousingInteriorPlacedItems12 as u16 as i32)
        );
        assert_eq!(result.rows.last().map(|row| row.slot), Some(49));
    }

    #[test]
    fn resolves_relative_paths_and_preset_names_under_layout_root() {
        let root =
            std::env::temp_dir().join(format!("kawari-remake-place-test-{}", std::process::id()));
        let preset_dir = root.join("CAFE CAT WALK");
        fs::create_dir_all(&preset_dir).unwrap();
        let preset_path = preset_dir.join("CAFE CAT WALK.json");
        fs::write(&preset_path, "{}").unwrap();

        let resolved = resolve_remake_place_preset_path_under_root("CAFE CAT WALK", &root).unwrap();
        assert_eq!(resolved.path, preset_path.canonicalize().unwrap());

        let resolved = resolve_remake_place_preset_path_under_root(
            r#"CAFE CAT WALK\CAFE CAT WALK.json"#,
            &root,
        )
        .unwrap();
        assert_eq!(resolved.path, preset_path.canonicalize().unwrap());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolves_absolute_json_paths_outside_layout_root() {
        let root = std::env::temp_dir().join(format!(
            "kawari-remake-place-missing-root-test-{}",
            std::process::id()
        ));
        let external_root = std::env::temp_dir().join(format!(
            "kawari-remake-place-absolute-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&external_root).unwrap();
        let preset_path = external_root.join("outside.json");
        fs::write(&preset_path, "{}").unwrap();

        let resolved =
            resolve_remake_place_preset_path_under_root(&preset_path.display().to_string(), &root)
                .unwrap();

        assert_eq!(resolved.path, preset_path.canonicalize().unwrap());
        assert_eq!(resolved.root, external_root.canonicalize().unwrap());

        fs::remove_dir_all(&external_root).unwrap();
    }

    #[test]
    fn parses_utf16_le_remake_place_preset_files() {
        let root = std::env::temp_dir().join(format!(
            "kawari-remake-place-utf16-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let preset_path = root.join("utf16.json");
        let json = r#"{"interiorFurniture":[{"itemId":1001}]}"#;
        let mut bytes = vec![0xFF, 0xFE];
        for word in json.encode_utf16() {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        fs::write(&preset_path, bytes).unwrap();

        let layout = parse_remake_place_layout_file(&preset_path).unwrap();

        assert_eq!(layout.interior_furniture.len(), 1);
        assert_eq!(layout.interior_furniture[0].item_id, 1001);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_ambiguous_directory_name_matches() {
        let root = std::env::temp_dir().join(format!(
            "kawari-remake-place-ambiguous-test-{}",
            std::process::id()
        ));
        let preset_dir = root.join("Modern Mansion [L]");
        fs::create_dir_all(&preset_dir).unwrap();
        fs::write(preset_dir.join("modernmansionmist.json"), "{}").unwrap();
        fs::write(preset_dir.join("modernmansionshirofixed.json"), "{}").unwrap();

        let error =
            resolve_remake_place_preset_path_under_root("Modern Mansion [L]", &root).unwrap_err();

        assert!(error.contains("ambiguous"));
        assert!(error.contains("modernmansionmist.json"));
        assert!(error.contains("modernmansionshirofixed.json"));

        fs::remove_dir_all(&root).unwrap();
    }
}
