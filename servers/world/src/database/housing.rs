use diesel::{SqliteConnection, prelude::*};
use kawari::{
    common::{ContainerType, HouseId, HouseUnit, Position},
    ipc::zone::PlotSize,
};
use serde::{Deserialize, Serialize};

use crate::{
    housing::{
        admin::{
            HousingAdminEstateSummaryRow, HousingAdminFurnitureRow, HousingEstateAdminDetail,
            HousingFurnitureCounts,
        },
        apartment::valid_apartment_room_number,
        constants::{
            DEFAULT_LOCAL_HOUSING_DIVISION, DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
            DEFAULT_LOCAL_HOUSING_PLOT_INDEX, DEFAULT_LOCAL_HOUSING_PLOT_SIZE,
            DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID, DEFAULT_LOCAL_HOUSING_WARD_INDEX,
        },
        container::{container_type_to_i32, housing_container_kind},
        text::truncate_utf8_to_raw_bytes,
    },
    inventory::interior_placed_containers,
};

use super::{
    WorldDatabase,
    models::{HousingEstate, HousingFurniture},
    schema::{housing_estates, housing_furniture},
    unixepoch,
};

const HOUSING_ESTATE_NAME_MAX_PAYLOAD_BYTES: usize = 20;
const HOUSING_GREETING_MAX_PAYLOAD_BYTES: usize = 192;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HousingEstateExport {
    pub estate: HousingEstate,
    pub furniture: Vec<HousingFurniture>,
}

#[derive(Clone, Debug)]
pub struct HousingEstateSpec {
    pub owner_content_id: u64,
    pub owner_name: String,
    pub world_id: u16,
    pub territory_type_id: u16,
    pub ward_index: u8,
    pub division: u8,
    pub plot_index: u8,
    pub plot_size: PlotSize,
    pub free_company: bool,
}

impl WorldDatabase {
    pub fn ensure_local_estate(
        &mut self,
        for_owner_content_id: u64,
        for_owner_name: &str,
        for_world_id: u16,
    ) -> HousingEstate {
        self.ensure_local_estate_with_spec(HousingEstateSpec {
            owner_content_id: for_owner_content_id,
            owner_name: for_owner_name.to_string(),
            world_id: for_world_id,
            territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
            plot_index: DEFAULT_LOCAL_HOUSING_PLOT_INDEX,
            plot_size: DEFAULT_LOCAL_HOUSING_PLOT_SIZE,
            free_company: false,
        })
    }

    pub fn ensure_local_estate_with_spec(&mut self, spec: HousingEstateSpec) -> HousingEstate {
        let now = self.database_time();
        let house_id = local_estate_house_id(&spec);
        let land_ident = house_id.to_u64() as i64;

        let flags = DEFAULT_LOCAL_HOUSING_LAND_FLAGS | if spec.free_company { 0x10 } else { 0 };
        let estate = HousingEstate {
            land_ident,
            house_id: house_id.to_u64() as i64,
            territory_type_id: spec.territory_type_id as i32,
            world_id: spec.world_id as i32,
            ward_index: spec.ward_index as i32,
            division: spec.division as i32,
            plot_index: spec.plot_index as i32,
            room_number: 0,
            is_apartment: false,
            owner_content_id: Some(spec.owner_content_id as i64),
            owner_name: spec.owner_name.clone(),
            plot_size: spec.plot_size as i32,
            flags,
            estate_name: truncate_utf8_to_raw_bytes(
                &format!("{}'s Local Estate", spec.owner_name),
                HOUSING_ESTATE_NAME_MAX_PAYLOAD_BYTES,
            ),
            greeting: truncate_utf8_to_raw_bytes(
                "A local Kawari debug estate.",
                HOUSING_GREETING_MAX_PAYLOAD_BYTES,
            ),
            exterior_json: "{}".to_string(),
            interior_json: "{}".to_string(),
            light_level: 0,
            created_at: now,
            updated_at: now,
        };

        self.connection
            .transaction::<HousingEstate, diesel::result::Error, _>(|connection| {
                let stale_estates = housing_estates::table
                    .select(HousingEstate::as_select())
                    .filter(housing_estates::owner_content_id.eq(spec.owner_content_id as i64))
                    .filter(housing_estates::territory_type_id.eq(spec.territory_type_id as i32))
                    .filter(housing_estates::world_id.eq(spec.world_id as i32))
                    .filter(housing_estates::ward_index.eq(spec.ward_index as i32))
                    .filter(housing_estates::division.eq(spec.division as i32))
                    .filter(housing_estates::is_apartment.eq(false))
                    .filter(housing_estates::room_number.eq(0))
                    .filter(housing_estates::plot_index.ne(spec.plot_index as i32))
                    .order(housing_estates::land_ident.desc())
                    .load(connection)?;

                for stale_estate in stale_estates {
                    migrate_housing_furniture_land_ident_on_connection(
                        connection,
                        stale_estate.land_ident,
                        land_ident,
                        now,
                    )?;
                }

                diesel::delete(
                    housing_estates::table
                        .filter(housing_estates::owner_content_id.eq(spec.owner_content_id as i64))
                        .filter(
                            housing_estates::territory_type_id.eq(spec.territory_type_id as i32),
                        )
                        .filter(housing_estates::world_id.eq(spec.world_id as i32))
                        .filter(housing_estates::ward_index.eq(spec.ward_index as i32))
                        .filter(housing_estates::division.eq(spec.division as i32))
                        .filter(housing_estates::is_apartment.eq(false))
                        .filter(housing_estates::room_number.eq(0))
                        .filter(housing_estates::plot_index.ne(spec.plot_index as i32)),
                )
                .execute(connection)?;

                diesel::insert_into(housing_estates::table)
                    .values(&estate)
                    .on_conflict(housing_estates::land_ident)
                    .do_update()
                    .set((
                        housing_estates::owner_content_id.eq(estate.owner_content_id),
                        housing_estates::owner_name.eq(estate.owner_name.clone()),
                        housing_estates::estate_name.eq(estate.estate_name.clone()),
                        housing_estates::greeting.eq(estate.greeting.clone()),
                        housing_estates::plot_size.eq(estate.plot_size),
                        housing_estates::flags.eq(estate.flags),
                        housing_estates::territory_type_id.eq(estate.territory_type_id),
                        housing_estates::world_id.eq(estate.world_id),
                        housing_estates::ward_index.eq(estate.ward_index),
                        housing_estates::division.eq(estate.division),
                        housing_estates::plot_index.eq(estate.plot_index),
                        housing_estates::updated_at.eq(now),
                    ))
                    .execute(connection)?;

                housing_estates::table
                    .select(HousingEstate::as_select())
                    .filter(housing_estates::land_ident.eq(land_ident))
                    .first(connection)
            })
            .unwrap()
    }

    pub fn ensure_local_apartment(
        &mut self,
        for_owner_content_id: u64,
        for_owner_name: &str,
        for_world_id: u16,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
        room_number: u16,
    ) -> Option<HousingEstate> {
        if !valid_apartment_room_number(room_number) {
            return None;
        }

        let now = self.database_time();
        let house_id = local_apartment_house_id(
            territory_type_id,
            for_world_id,
            ward_index,
            division,
            room_number,
        );
        let land_ident = house_id.to_u64() as i64;
        let estate = HousingEstate {
            land_ident,
            house_id: land_ident,
            territory_type_id: territory_type_id as i32,
            world_id: for_world_id as i32,
            ward_index: ward_index as i32,
            division: division as i32,
            plot_index: 0,
            room_number: room_number as i32,
            is_apartment: true,
            owner_content_id: Some(for_owner_content_id as i64),
            owner_name: for_owner_name.to_string(),
            plot_size: PlotSize::Small as i32,
            flags: DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
            estate_name: truncate_utf8_to_raw_bytes(
                &format!("{for_owner_name}'s Apt. {room_number}"),
                HOUSING_ESTATE_NAME_MAX_PAYLOAD_BYTES,
            ),
            greeting: truncate_utf8_to_raw_bytes(
                "A local Kawari debug apartment.",
                HOUSING_GREETING_MAX_PAYLOAD_BYTES,
            ),
            exterior_json: "{}".to_string(),
            interior_json: "{}".to_string(),
            light_level: 0,
            created_at: now,
            updated_at: now,
        };

        diesel::insert_into(housing_estates::table)
            .values(&estate)
            .on_conflict(housing_estates::land_ident)
            .do_update()
            .set((
                housing_estates::owner_content_id.eq(estate.owner_content_id),
                housing_estates::owner_name.eq(estate.owner_name.clone()),
                housing_estates::estate_name.eq(estate.estate_name.clone()),
                housing_estates::greeting.eq(estate.greeting.clone()),
                housing_estates::territory_type_id.eq(estate.territory_type_id),
                housing_estates::world_id.eq(estate.world_id),
                housing_estates::ward_index.eq(estate.ward_index),
                housing_estates::division.eq(estate.division),
                housing_estates::plot_index.eq(estate.plot_index),
                housing_estates::room_number.eq(estate.room_number),
                housing_estates::is_apartment.eq(estate.is_apartment),
                housing_estates::flags.eq(estate.flags),
                housing_estates::updated_at.eq(now),
            ))
            .execute(&mut self.connection)
            .unwrap();

        housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::land_ident.eq(land_ident))
            .first(&mut self.connection)
            .ok()
    }

    pub fn owned_housing_estates(&mut self, for_owner_content_id: u64) -> Vec<HousingEstate> {
        housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::owner_content_id.eq(for_owner_content_id as i64))
            .order((
                housing_estates::is_apartment.asc(),
                housing_estates::territory_type_id.asc(),
                housing_estates::ward_index.asc(),
                housing_estates::division.asc(),
                housing_estates::plot_index.asc(),
                housing_estates::room_number.asc(),
                housing_estates::land_ident.asc(),
            ))
            .load(&mut self.connection)
            .unwrap_or_default()
    }

    pub fn housing_estate_by_house_id(&mut self, id: HouseId) -> Option<HousingEstate> {
        housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::house_id.eq(id.to_u64() as i64))
            .first(&mut self.connection)
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn insert_housing_estate_for_test(
        &mut self,
        estate: HousingEstate,
    ) -> HousingEstate {
        diesel::insert_into(housing_estates::table)
            .values(&estate)
            .execute(&mut self.connection)
            .unwrap();

        estate
    }

    pub fn housing_estate_by_location(
        &mut self,
        territory_type_id: u16,
        world_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: u8,
    ) -> Option<HousingEstate> {
        housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::territory_type_id.eq(territory_type_id as i32))
            .filter(housing_estates::world_id.eq(world_id as i32))
            .filter(housing_estates::ward_index.eq(ward_index as i32))
            .filter(housing_estates::division.eq(division as i32))
            .filter(housing_estates::plot_index.eq(plot_index as i32))
            .first(&mut self.connection)
            .ok()
    }

    pub fn housing_estates_by_ward(
        &mut self,
        territory_type_id: u16,
        world_id: u16,
        ward_index: u8,
        division: u8,
    ) -> Vec<HousingEstate> {
        housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::territory_type_id.eq(territory_type_id as i32))
            .filter(housing_estates::world_id.eq(world_id as i32))
            .filter(housing_estates::ward_index.eq(ward_index as i32))
            .filter(housing_estates::division.eq(division as i32))
            .filter(housing_estates::is_apartment.eq(false))
            .filter(housing_estates::room_number.eq(0))
            .order(housing_estates::plot_index.asc())
            .load(&mut self.connection)
            .unwrap_or_default()
    }

    pub fn housing_estates_by_ward_and_divisions(
        &mut self,
        territory_type_id: u16,
        world_id: u16,
        ward_index: u8,
    ) -> (Vec<HousingEstate>, Vec<HousingEstate>) {
        (
            self.housing_estates_by_ward(territory_type_id, world_id, ward_index, 0),
            self.housing_estates_by_ward(territory_type_id, world_id, ward_index, 1),
        )
    }

    pub fn housing_apartments_by_ward(
        &mut self,
        territory_type_id: u16,
        world_id: u16,
        ward_index: u8,
        division: u8,
    ) -> Vec<HousingEstate> {
        housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::territory_type_id.eq(territory_type_id as i32))
            .filter(housing_estates::world_id.eq(world_id as i32))
            .filter(housing_estates::ward_index.eq(ward_index as i32))
            .filter(housing_estates::division.eq(division as i32))
            .filter(housing_estates::is_apartment.eq(true))
            .filter(housing_estates::room_number.gt(0))
            .order((
                housing_estates::room_number.asc(),
                housing_estates::land_ident.asc(),
            ))
            .load(&mut self.connection)
            .unwrap_or_default()
    }

    pub fn housing_apartment_by_room(
        &mut self,
        territory_type_id: u16,
        world_id: u16,
        ward_index: u8,
        division: u8,
        room_number: u16,
    ) -> Option<HousingEstate> {
        if !valid_apartment_room_number(room_number) {
            return None;
        }

        housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::territory_type_id.eq(territory_type_id as i32))
            .filter(housing_estates::world_id.eq(world_id as i32))
            .filter(housing_estates::ward_index.eq(ward_index as i32))
            .filter(housing_estates::division.eq(division as i32))
            .filter(housing_estates::is_apartment.eq(true))
            .filter(housing_estates::room_number.eq(room_number as i32))
            .first(&mut self.connection)
            .ok()
    }

    pub fn update_housing_light_level(&mut self, for_land_ident: i64, level: u8) -> bool {
        diesel::update(
            housing_estates::table.filter(housing_estates::land_ident.eq(for_land_ident)),
        )
        .set((
            housing_estates::light_level.eq(level as i32),
            housing_estates::updated_at.eq(self.database_time()),
        ))
        .execute(&mut self.connection)
        .unwrap_or_default()
            > 0
    }

    pub fn update_housing_name(&mut self, for_land_ident: i64, name: &str) -> bool {
        let name = truncate_utf8_to_raw_bytes(name, HOUSING_ESTATE_NAME_MAX_PAYLOAD_BYTES);

        diesel::update(
            housing_estates::table.filter(housing_estates::land_ident.eq(for_land_ident)),
        )
        .set((
            housing_estates::estate_name.eq(name),
            housing_estates::updated_at.eq(self.database_time()),
        ))
        .execute(&mut self.connection)
        .unwrap_or_default()
            > 0
    }

    pub fn update_housing_greeting(&mut self, for_land_ident: i64, greeting: &str) -> bool {
        let greeting = truncate_utf8_to_raw_bytes(greeting, HOUSING_GREETING_MAX_PAYLOAD_BYTES);

        diesel::update(
            housing_estates::table.filter(housing_estates::land_ident.eq(for_land_ident)),
        )
        .set((
            housing_estates::greeting.eq(greeting),
            housing_estates::updated_at.eq(self.database_time()),
        ))
        .execute(&mut self.connection)
        .unwrap_or_default()
            > 0
    }

    pub fn update_housing_exterior_json(
        &mut self,
        for_land_ident: i64,
        exterior_json: &str,
    ) -> bool {
        diesel::update(
            housing_estates::table.filter(housing_estates::land_ident.eq(for_land_ident)),
        )
        .set((
            housing_estates::exterior_json.eq(exterior_json),
            housing_estates::updated_at.eq(self.database_time()),
        ))
        .execute(&mut self.connection)
        .unwrap_or_default()
            > 0
    }

    pub fn update_housing_interior_json(
        &mut self,
        for_land_ident: i64,
        interior_json: &str,
    ) -> bool {
        diesel::update(
            housing_estates::table.filter(housing_estates::land_ident.eq(for_land_ident)),
        )
        .set((
            housing_estates::interior_json.eq(interior_json),
            housing_estates::updated_at.eq(self.database_time()),
        ))
        .execute(&mut self.connection)
        .unwrap_or_default()
            > 0
    }

    pub fn delete_housing_furniture_for_estate(&mut self, for_land_ident: i64) -> usize {
        diesel::delete(
            housing_furniture::table.filter(housing_furniture::land_ident.eq(for_land_ident)),
        )
        .execute(&mut self.connection)
        .unwrap_or_default()
    }

    pub fn replace_housing_placed_furniture_for_estate(
        &mut self,
        for_land_ident: i64,
        include_interior: bool,
        include_exterior: bool,
        rows: &[HousingFurniture],
    ) -> Result<usize, diesel::result::Error> {
        let now = self.database_time();

        self.connection.transaction(|connection| {
            let mut deleted = 0;

            if include_exterior {
                deleted += diesel::delete(
                    housing_furniture::table
                        .filter(housing_furniture::land_ident.eq(for_land_ident))
                        .filter(housing_furniture::container_type.eq(container_type_to_i32(
                            ContainerType::HousingExteriorPlacedItems,
                        ))),
                )
                .execute(connection)?;
            }

            if include_interior {
                for container in interior_placed_containers() {
                    deleted += diesel::delete(
                        housing_furniture::table
                            .filter(housing_furniture::land_ident.eq(for_land_ident))
                            .filter(
                                housing_furniture::container_type
                                    .eq(container_type_to_i32(container)),
                            ),
                    )
                    .execute(connection)?;
                }
            }

            for row in rows {
                let mut row = row.clone();
                row.updated_at = now;

                diesel::insert_into(housing_furniture::table)
                    .values(&row)
                    .on_conflict((
                        housing_furniture::land_ident,
                        housing_furniture::container_type,
                        housing_furniture::slot,
                    ))
                    .do_update()
                    .set((
                        housing_furniture::item_id.eq(row.item_id),
                        housing_furniture::catalog_id.eq(row.catalog_id),
                        housing_furniture::stain.eq(row.stain),
                        housing_furniture::placed.eq(row.placed),
                        housing_furniture::pos_x.eq(row.pos_x),
                        housing_furniture::pos_y.eq(row.pos_y),
                        housing_furniture::pos_z.eq(row.pos_z),
                        housing_furniture::rotation.eq(row.rotation),
                        housing_furniture::created_by_content_id.eq(row.created_by_content_id),
                        housing_furniture::updated_at.eq(row.updated_at),
                    ))
                    .execute(connection)?;
            }

            Ok(deleted)
        })
    }

    pub fn delete_housing_estate_and_furniture(&mut self, for_land_ident: i64) -> bool {
        self.connection
            .transaction::<bool, diesel::result::Error, _>(|connection| {
                diesel::delete(
                    housing_furniture::table
                        .filter(housing_furniture::land_ident.eq(for_land_ident)),
                )
                .execute(connection)?;

                let deleted_estates = diesel::delete(
                    housing_estates::table.filter(housing_estates::land_ident.eq(for_land_ident)),
                )
                .execute(connection)?;

                Ok(deleted_estates > 0)
            })
            .unwrap_or_default()
    }

    pub fn housing_summary_rows_for_admin(&mut self) -> Vec<HousingAdminEstateSummaryRow> {
        let estates = housing_estates::table
            .select(HousingEstate::as_select())
            .order((
                housing_estates::territory_type_id.asc(),
                housing_estates::world_id.asc(),
                housing_estates::ward_index.asc(),
                housing_estates::division.asc(),
                housing_estates::is_apartment.asc(),
                housing_estates::plot_index.asc(),
                housing_estates::room_number.asc(),
                housing_estates::land_ident.asc(),
            ))
            .load(&mut self.connection)
            .unwrap_or_default();

        estates
            .into_iter()
            .map(|estate| {
                let furniture = self.list_all_housing_furniture(estate.land_ident);
                HousingAdminEstateSummaryRow {
                    land_ident: estate.land_ident,
                    house_id: estate.house_id,
                    owner_content_id: estate.owner_content_id,
                    owner_name: estate.owner_name.clone(),
                    plot: housing_plot_label(&estate),
                    kind: housing_estate_kind(&estate).to_string(),
                    size: housing_estate_size(&estate).to_string(),
                    flags: estate.flags,
                    furniture_counts: summarize_housing_furniture_counts(&furniture),
                }
            })
            .collect()
    }

    pub fn housing_estate_detail_for_admin(
        &mut self,
        land_ident: i64,
    ) -> Option<HousingEstateAdminDetail> {
        let estate = housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::land_ident.eq(land_ident))
            .first(&mut self.connection)
            .ok()?;
        let furniture = self.list_all_housing_furniture(land_ident);
        let furniture_counts = summarize_housing_furniture_counts(&furniture);
        let furniture = furniture
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

        Some(HousingEstateAdminDetail {
            estate,
            furniture_counts,
            furniture,
        })
    }

    pub fn export_housing_estate(&mut self, land_ident: i64) -> Option<HousingEstateExport> {
        let estate = housing_estates::table
            .select(HousingEstate::as_select())
            .filter(housing_estates::land_ident.eq(land_ident))
            .first(&mut self.connection)
            .ok()?;

        Some(HousingEstateExport {
            estate,
            furniture: self.list_all_housing_furniture(land_ident),
        })
    }

    pub fn import_housing_estate(&mut self, export: HousingEstateExport) -> bool {
        let estate = export.estate;
        let furniture = export
            .furniture
            .into_iter()
            .map(|mut row| {
                row.land_ident = estate.land_ident;
                row
            })
            .collect::<Vec<_>>();

        self.connection
            .transaction::<bool, diesel::result::Error, _>(|connection| {
                diesel::insert_into(housing_estates::table)
                    .values(&estate)
                    .on_conflict(housing_estates::land_ident)
                    .do_update()
                    .set((
                        housing_estates::house_id.eq(estate.house_id),
                        housing_estates::territory_type_id.eq(estate.territory_type_id),
                        housing_estates::world_id.eq(estate.world_id),
                        housing_estates::ward_index.eq(estate.ward_index),
                        housing_estates::division.eq(estate.division),
                        housing_estates::plot_index.eq(estate.plot_index),
                        housing_estates::room_number.eq(estate.room_number),
                        housing_estates::is_apartment.eq(estate.is_apartment),
                        housing_estates::owner_content_id.eq(estate.owner_content_id),
                        housing_estates::owner_name.eq(estate.owner_name.clone()),
                        housing_estates::plot_size.eq(estate.plot_size),
                        housing_estates::flags.eq(estate.flags),
                        housing_estates::estate_name.eq(estate.estate_name.clone()),
                        housing_estates::greeting.eq(estate.greeting.clone()),
                        housing_estates::exterior_json.eq(estate.exterior_json.clone()),
                        housing_estates::interior_json.eq(estate.interior_json.clone()),
                        housing_estates::light_level.eq(estate.light_level),
                        housing_estates::created_at.eq(estate.created_at),
                        housing_estates::updated_at.eq(estate.updated_at),
                    ))
                    .execute(connection)?;

                diesel::delete(
                    housing_furniture::table
                        .filter(housing_furniture::land_ident.eq(estate.land_ident)),
                )
                .execute(connection)?;

                for row in &furniture {
                    diesel::insert_into(housing_furniture::table)
                        .values(row)
                        .on_conflict((
                            housing_furniture::land_ident,
                            housing_furniture::container_type,
                            housing_furniture::slot,
                        ))
                        .do_update()
                        .set((
                            housing_furniture::item_id.eq(row.item_id),
                            housing_furniture::catalog_id.eq(row.catalog_id),
                            housing_furniture::stain.eq(row.stain),
                            housing_furniture::placed.eq(row.placed),
                            housing_furniture::pos_x.eq(row.pos_x),
                            housing_furniture::pos_y.eq(row.pos_y),
                            housing_furniture::pos_z.eq(row.pos_z),
                            housing_furniture::rotation.eq(row.rotation),
                            housing_furniture::created_by_content_id.eq(row.created_by_content_id),
                            housing_furniture::updated_at.eq(row.updated_at),
                        ))
                        .execute(connection)?;
                }

                Ok(true)
            })
            .unwrap_or_default()
    }

    pub fn list_housing_furniture(
        &mut self,
        for_land_ident: i64,
        placed: bool,
    ) -> Vec<HousingFurniture> {
        housing_furniture::table
            .select(HousingFurniture::as_select())
            .filter(housing_furniture::land_ident.eq(for_land_ident))
            .filter(housing_furniture::placed.eq(placed))
            .order((
                housing_furniture::container_type.asc(),
                housing_furniture::slot.asc(),
            ))
            .load(&mut self.connection)
            .unwrap_or_default()
    }

    pub fn list_all_housing_furniture(&mut self, for_land_ident: i64) -> Vec<HousingFurniture> {
        housing_furniture::table
            .select(HousingFurniture::as_select())
            .filter(housing_furniture::land_ident.eq(for_land_ident))
            .order((
                housing_furniture::container_type.asc(),
                housing_furniture::slot.asc(),
            ))
            .load(&mut self.connection)
            .unwrap_or_default()
    }

    pub fn upsert_housing_furniture(&mut self, mut row: HousingFurniture) {
        row.updated_at = self.database_time();

        diesel::insert_into(housing_furniture::table)
            .values(&row)
            .on_conflict((
                housing_furniture::land_ident,
                housing_furniture::container_type,
                housing_furniture::slot,
            ))
            .do_update()
            .set((
                housing_furniture::item_id.eq(row.item_id),
                housing_furniture::catalog_id.eq(row.catalog_id),
                housing_furniture::stain.eq(row.stain),
                housing_furniture::placed.eq(row.placed),
                housing_furniture::pos_x.eq(row.pos_x),
                housing_furniture::pos_y.eq(row.pos_y),
                housing_furniture::pos_z.eq(row.pos_z),
                housing_furniture::rotation.eq(row.rotation),
                housing_furniture::created_by_content_id.eq(row.created_by_content_id),
                housing_furniture::updated_at.eq(row.updated_at),
            ))
            .execute(&mut self.connection)
            .unwrap();
    }

    pub fn update_housing_furniture_position(
        &mut self,
        for_land_ident: i64,
        for_container_type: ContainerType,
        for_slot: u16,
        position: Position,
        rotation: f32,
    ) -> bool {
        diesel::update(
            housing_furniture::table
                .filter(housing_furniture::land_ident.eq(for_land_ident))
                .filter(
                    housing_furniture::container_type.eq(container_type_to_i32(for_container_type)),
                )
                .filter(housing_furniture::slot.eq(for_slot as i32)),
        )
        .set((
            housing_furniture::pos_x.eq(position.0.x),
            housing_furniture::pos_y.eq(position.0.y),
            housing_furniture::pos_z.eq(position.0.z),
            housing_furniture::rotation.eq(rotation),
            housing_furniture::updated_at.eq(self.database_time()),
        ))
        .execute(&mut self.connection)
        .unwrap_or_default()
            > 0
    }

    pub fn delete_housing_furniture_slot(
        &mut self,
        for_land_ident: i64,
        for_container_type: ContainerType,
        for_slot: u16,
    ) -> bool {
        diesel::delete(
            housing_furniture::table
                .filter(housing_furniture::land_ident.eq(for_land_ident))
                .filter(
                    housing_furniture::container_type.eq(container_type_to_i32(for_container_type)),
                )
                .filter(housing_furniture::slot.eq(for_slot as i32)),
        )
        .execute(&mut self.connection)
        .unwrap_or_default()
            > 0
    }

    pub fn move_housing_furniture_to_container(
        &mut self,
        for_land_ident: i64,
        src_container: ContainerType,
        src_slot: u16,
        dst_container: Option<ContainerType>,
        dst_slot: Option<u16>,
        placed: bool,
    ) -> bool {
        let now = self.database_time();
        let src_container = container_type_to_i32(src_container);

        self.connection
            .transaction::<bool, diesel::result::Error, _>(|connection| {
                let Some(mut row) = housing_furniture::table
                    .select(HousingFurniture::as_select())
                    .filter(housing_furniture::land_ident.eq(for_land_ident))
                    .filter(housing_furniture::container_type.eq(src_container))
                    .filter(housing_furniture::slot.eq(src_slot as i32))
                    .first(connection)
                    .optional()?
                else {
                    return Ok(false);
                };

                diesel::delete(
                    housing_furniture::table
                        .filter(housing_furniture::land_ident.eq(for_land_ident))
                        .filter(housing_furniture::container_type.eq(src_container))
                        .filter(housing_furniture::slot.eq(src_slot as i32)),
                )
                .execute(connection)?;

                if let (Some(dst_container), Some(dst_slot)) = (dst_container, dst_slot) {
                    row.container_type = container_type_to_i32(dst_container);
                    row.slot = dst_slot as i32;
                    row.placed = placed;
                    row.updated_at = now;

                    diesel::insert_into(housing_furniture::table)
                        .values(&row)
                        .on_conflict((
                            housing_furniture::land_ident,
                            housing_furniture::container_type,
                            housing_furniture::slot,
                        ))
                        .do_update()
                        .set((
                            housing_furniture::item_id.eq(row.item_id),
                            housing_furniture::catalog_id.eq(row.catalog_id),
                            housing_furniture::stain.eq(row.stain),
                            housing_furniture::placed.eq(row.placed),
                            housing_furniture::pos_x.eq(row.pos_x),
                            housing_furniture::pos_y.eq(row.pos_y),
                            housing_furniture::pos_z.eq(row.pos_z),
                            housing_furniture::rotation.eq(row.rotation),
                            housing_furniture::created_by_content_id.eq(row.created_by_content_id),
                            housing_furniture::updated_at.eq(row.updated_at),
                        ))
                        .execute(connection)?;
                }

                Ok(true)
            })
            .unwrap_or_default()
    }

    fn database_time(&mut self) -> i64 {
        diesel::select(unixepoch())
            .get_result::<i64>(&mut self.connection)
            .unwrap_or_default()
    }
}

fn migrate_housing_furniture_land_ident_on_connection(
    connection: &mut SqliteConnection,
    from_land_ident: i64,
    to_land_ident: i64,
    updated_at: i64,
) -> QueryResult<()> {
    if from_land_ident == to_land_ident {
        return Ok(());
    }

    let rows = housing_furniture::table
        .select(HousingFurniture::as_select())
        .filter(housing_furniture::land_ident.eq(from_land_ident))
        .load(connection)?;

    for mut row in rows {
        row.land_ident = to_land_ident;
        row.updated_at = updated_at;

        diesel::insert_into(housing_furniture::table)
            .values(&row)
            .on_conflict((
                housing_furniture::land_ident,
                housing_furniture::container_type,
                housing_furniture::slot,
            ))
            .do_update()
            .set((
                housing_furniture::item_id.eq(row.item_id),
                housing_furniture::catalog_id.eq(row.catalog_id),
                housing_furniture::stain.eq(row.stain),
                housing_furniture::placed.eq(row.placed),
                housing_furniture::pos_x.eq(row.pos_x),
                housing_furniture::pos_y.eq(row.pos_y),
                housing_furniture::pos_z.eq(row.pos_z),
                housing_furniture::rotation.eq(row.rotation),
                housing_furniture::created_by_content_id.eq(row.created_by_content_id),
                housing_furniture::updated_at.eq(row.updated_at),
            ))
            .execute(connection)?;
    }

    diesel::delete(
        housing_furniture::table.filter(housing_furniture::land_ident.eq(from_land_ident)),
    )
    .execute(connection)?;

    Ok(())
}

fn summarize_housing_furniture_counts(rows: &[HousingFurniture]) -> HousingFurnitureCounts {
    let mut counts = HousingFurnitureCounts::default();
    counts.total = rows.len();

    for row in rows {
        match housing_container_kind(row.container_type) {
            "indoor_placed" => counts.indoor_placed += 1,
            "indoor_storeroom" => counts.indoor_storeroom += 1,
            "outdoor_placed" => counts.outdoor_placed += 1,
            "outdoor_storeroom" => counts.outdoor_storeroom += 1,
            _ => {}
        }
    }

    counts
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

fn local_estate_house_id(spec: &HousingEstateSpec) -> HouseId {
    HouseId {
        unit: HouseUnit {
            apartment_division_plot_index: spec.plot_index + spec.division * 30,
            apartment_flag: false,
        },
        unk1: 0,
        ward_index: spec.ward_index,
        room_number: 0,
        territory_type_id: spec.territory_type_id,
        world_id: spec.world_id,
    }
}

fn local_apartment_house_id(
    territory_type_id: u16,
    world_id: u16,
    ward_index: u8,
    division: u8,
    room_number: u16,
) -> HouseId {
    HouseId {
        unit: HouseUnit {
            apartment_division_plot_index: division,
            apartment_flag: true,
        },
        unk1: 0,
        ward_index,
        room_number,
        territory_type_id,
        world_id,
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    fn test_db() -> WorldDatabase {
        WorldDatabase::new_at(":memory:")
    }

    #[test]
    fn ensure_local_estate_creates_one_estate() {
        let mut db = test_db();

        let estate = db.ensure_local_estate(100, "Tester", 67);

        assert_eq!(estate.owner_content_id, Some(100));
        assert_eq!(estate.owner_name, "Tester");
        assert_eq!(
            estate.territory_type_id,
            DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32
        );
        assert_eq!(estate.ward_index, DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32);
        assert_eq!(estate.division, DEFAULT_LOCAL_HOUSING_DIVISION as i32);
        assert_eq!(estate.plot_index, DEFAULT_LOCAL_HOUSING_PLOT_INDEX as i32);
        assert_eq!(estate.plot_size, DEFAULT_LOCAL_HOUSING_PLOT_SIZE as i32);
        assert_eq!(estate.flags, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
        assert_eq!(estate.house_id, estate.land_ident);
    }

    #[test]
    fn local_estate_land_flags_mark_personal_built_house_without_fc() {
        assert_eq!(DEFAULT_LOCAL_HOUSING_LAND_FLAGS & 0x01, 0x01);
        assert_eq!(DEFAULT_LOCAL_HOUSING_LAND_FLAGS & 0x02, 0x02);
        assert_eq!(DEFAULT_LOCAL_HOUSING_LAND_FLAGS & 0x08, 0x08);
        assert_eq!(DEFAULT_LOCAL_HOUSING_LAND_FLAGS & 0x10, 0x00);
        assert_eq!(DEFAULT_LOCAL_HOUSING_LAND_FLAGS, 0x0B);
    }

    #[test]
    fn ensure_local_estate_accepts_fc_large_plot_options() {
        let mut db = test_db();

        let estate = db.ensure_local_estate_with_spec(HousingEstateSpec {
            owner_content_id: 100,
            owner_name: "Tester FC".to_string(),
            world_id: 67,
            territory_type_id: 341,
            ward_index: 2,
            division: 1,
            plot_index: 12,
            plot_size: PlotSize::Large,
            free_company: true,
        });

        let house_id = HouseId::from_u64(estate.house_id as u64);

        assert_eq!(estate.owner_content_id, Some(100));
        assert_eq!(estate.owner_name, "Tester FC");
        assert_eq!(estate.territory_type_id, 341);
        assert_eq!(estate.world_id, 67);
        assert_eq!(estate.ward_index, 2);
        assert_eq!(estate.division, 1);
        assert_eq!(estate.plot_index, 12);
        assert_eq!(estate.plot_size, PlotSize::Large as i32);
        assert_eq!(estate.flags, DEFAULT_LOCAL_HOUSING_LAND_FLAGS | 0x10);
        assert_eq!(estate.house_id, estate.land_ident);
        assert_eq!(house_id.territory_type_id, 341);
        assert_eq!(house_id.world_id, 67);
        assert_eq!(house_id.ward_index, 2);
        assert_eq!(house_id.unit.apartment_division_plot_index, 42);
        assert!(!house_id.unit.apartment_flag);
    }

    #[test]
    fn ensure_local_estate_is_idempotent() {
        let mut db = test_db();

        let first = db.ensure_local_estate(100, "Tester", 67);
        let second = db.ensure_local_estate(100, "Tester Prime", 67);
        let estates = db.owned_housing_estates(100);

        assert_eq!(first.land_ident, second.land_ident);
        assert_eq!(estates.len(), 1);
        assert_eq!(estates[0].owner_name, "Tester Prime");
        assert_eq!(estates[0].plot_size, PlotSize::Large as i32);
        assert_eq!(estates[0].flags, DEFAULT_LOCAL_HOUSING_LAND_FLAGS);
    }

    #[test]
    fn ensure_local_estate_replaces_previous_local_plot() {
        let mut db = test_db();

        let old_house_id = HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: 4,
                apartment_flag: false,
            },
            unk1: 0,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            room_number: 0,
            territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            world_id: 67,
        };
        diesel::insert_into(housing_estates::table)
            .values(HousingEstate {
                land_ident: old_house_id.to_u64() as i64,
                house_id: old_house_id.to_u64() as i64,
                territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                world_id: 67,
                ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32,
                division: DEFAULT_LOCAL_HOUSING_DIVISION as i32,
                plot_index: 4,
                owner_content_id: Some(100),
                owner_name: "Tester".to_string(),
                estate_name: "Tester Old Estate".to_string(),
                ..Default::default()
            })
            .execute(&mut db.connection)
            .unwrap();

        let estate = db.ensure_local_estate(100, "Tester", 67);
        let estates = db.owned_housing_estates(100);

        assert_eq!(estate.plot_index, DEFAULT_LOCAL_HOUSING_PLOT_INDEX as i32);
        assert_eq!(estates.len(), 1);
        assert_eq!(
            estates[0].plot_index,
            DEFAULT_LOCAL_HOUSING_PLOT_INDEX as i32
        );
    }

    fn local_house_id(plot_index: u8) -> HouseId {
        HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: plot_index,
                apartment_flag: false,
            },
            unk1: 0,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            room_number: 0,
            territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            world_id: 67,
        }
    }

    fn insert_stale_local_estate(
        db: &mut WorldDatabase,
        owner_content_id: u64,
        plot_index: u8,
    ) -> i64 {
        let old_house_id = local_house_id(plot_index);
        let old_land_ident = old_house_id.to_u64() as i64;

        diesel::insert_into(housing_estates::table)
            .values(HousingEstate {
                land_ident: old_land_ident,
                house_id: old_land_ident,
                territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                world_id: 67,
                ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32,
                division: DEFAULT_LOCAL_HOUSING_DIVISION as i32,
                plot_index: plot_index as i32,
                owner_content_id: Some(owner_content_id as i64),
                owner_name: "Tester".to_string(),
                estate_name: "Tester Old Estate".to_string(),
                ..Default::default()
            })
            .execute(&mut db.connection)
            .unwrap();

        old_land_ident
    }

    #[test]
    fn ensure_local_estate_migrates_previous_local_plot_furniture() {
        let mut db = test_db();

        let old_house_id = HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: 4,
                apartment_flag: false,
            },
            unk1: 0,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            room_number: 0,
            territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            world_id: 67,
        };
        let old_land_ident = old_house_id.to_u64() as i64;

        diesel::insert_into(housing_estates::table)
            .values(HousingEstate {
                land_ident: old_land_ident,
                house_id: old_land_ident,
                territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                world_id: 67,
                ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32,
                division: DEFAULT_LOCAL_HOUSING_DIVISION as i32,
                plot_index: 4,
                owner_content_id: Some(100),
                owner_name: "Tester".to_string(),
                estate_name: "Tester Old Estate".to_string(),
                ..Default::default()
            })
            .execute(&mut db.connection)
            .unwrap();

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: old_land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 1000,
            catalog_id: 55,
            placed: true,
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: old_land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorStoreroom1),
            slot: 1,
            item_id: 1001,
            catalog_id: 56,
            placed: false,
            ..Default::default()
        });

        let estate = db.ensure_local_estate(100, "Tester", 67);

        assert!(db.list_housing_furniture(old_land_ident, true).is_empty());
        assert!(db.list_housing_furniture(old_land_ident, false).is_empty());
        assert_eq!(
            db.list_housing_furniture(estate.land_ident, true)[0].item_id,
            1000
        );
        assert_eq!(
            db.list_housing_furniture(estate.land_ident, false)[0].item_id,
            1001
        );
    }

    #[test]
    fn ensure_local_estate_migrates_stale_collision_in_descending_land_ident_order() {
        let mut db = test_db();
        let higher_land_ident = insert_stale_local_estate(&mut db, 100, 4);
        let lower_land_ident = insert_stale_local_estate(&mut db, 100, 3);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: higher_land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 4000,
            catalog_id: 80,
            placed: true,
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: lower_land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 3000,
            catalog_id: 70,
            placed: true,
            ..Default::default()
        });

        let estate = db.ensure_local_estate(100, "Tester", 67);
        let rows = db.list_housing_furniture(estate.land_ident, true);

        assert!(db.list_all_housing_furniture(higher_land_ident).is_empty());
        assert!(db.list_all_housing_furniture(lower_land_ident).is_empty());
        assert!(
            db.housing_estate_by_house_id(HouseId::from_u64(higher_land_ident as u64))
                .is_none()
        );
        assert!(
            db.housing_estate_by_house_id(HouseId::from_u64(lower_land_ident as u64))
                .is_none()
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].item_id, 3000);
        assert_eq!(rows[0].catalog_id, 70);
    }

    #[test]
    fn ensure_local_estate_does_not_delete_or_migrate_owned_apartment_rows() {
        let mut db = test_db();
        let apartment_house_id = HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: 4,
                apartment_flag: true,
            },
            unk1: 0,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            room_number: 12,
            territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            world_id: 67,
        };
        let apartment_land_ident = apartment_house_id.to_u64() as i64;

        diesel::insert_into(housing_estates::table)
            .values(HousingEstate {
                land_ident: apartment_land_ident,
                house_id: apartment_land_ident,
                territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                world_id: 67,
                ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32,
                division: DEFAULT_LOCAL_HOUSING_DIVISION as i32,
                plot_index: 4,
                room_number: 12,
                is_apartment: true,
                owner_content_id: Some(100),
                owner_name: "Tester".to_string(),
                estate_name: "Tester Apartment".to_string(),
                ..Default::default()
            })
            .execute(&mut db.connection)
            .unwrap();

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: apartment_land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 2000,
            catalog_id: 65,
            placed: true,
            ..Default::default()
        });

        let estate = db.ensure_local_estate(100, "Tester", 67);

        let apartment = db.housing_estate_by_house_id(apartment_house_id);
        assert!(apartment.is_some(), "apartment row must not be deleted");
        assert_eq!(
            db.list_housing_furniture(apartment_land_ident, true)[0].item_id,
            2000,
            "apartment furniture must stay on the apartment row"
        );
        assert!(
            db.list_housing_furniture(estate.land_ident, true)
                .iter()
                .all(|row| row.item_id != 2000),
            "apartment furniture must not migrate into the outdoor local estate"
        );
    }

    #[test]
    fn ensure_local_estate_preserves_non_outdoor_owned_rows() {
        let mut db = test_db();
        let non_outdoor_house_id = HouseId {
            unit: HouseUnit {
                apartment_division_plot_index: 4,
                apartment_flag: false,
            },
            unk1: 0,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX as u8,
            room_number: 12,
            territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            world_id: 67,
        };

        diesel::insert_into(housing_estates::table)
            .values(HousingEstate {
                land_ident: non_outdoor_house_id.to_u64() as i64,
                house_id: non_outdoor_house_id.to_u64() as i64,
                territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                world_id: 67,
                ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX as i32,
                division: DEFAULT_LOCAL_HOUSING_DIVISION as i32,
                plot_index: 9,
                room_number: 12,
                is_apartment: false,
                owner_content_id: Some(100),
                owner_name: "Tester".to_string(),
                estate_name: "Non-Outdoors Row".to_string(),
                ..Default::default()
            })
            .execute(&mut db.connection)
            .unwrap();
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: non_outdoor_house_id.to_u64() as i64,
            container_type: container_type_to_i32(ContainerType::HousingExteriorPlacedItems),
            slot: 0,
            item_id: 3000,
            catalog_id: 77,
            placed: true,
            ..Default::default()
        });

        let estate = db.ensure_local_estate(100, "Tester", 67);

        let preserved = db
            .housing_estate_by_house_id(non_outdoor_house_id)
            .expect("non-outdoor owned row should survive ensure_local_estate cleanup");
        let preserved_furniture = db.list_housing_furniture(preserved.land_ident, true);

        assert_eq!(preserved_furniture.len(), 1);
        assert_eq!(preserved_furniture[0].item_id, 3000);
        assert!(
            db.list_housing_furniture(estate.land_ident, true)
                .iter()
                .all(|row| row.item_id != 3000)
        );
    }

    #[test]
    fn update_housing_light_level_persists_for_estate() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        assert!(db.update_housing_light_level(estate.land_ident, 4));

        let updated = db.housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64));
        assert_eq!(updated.unwrap().light_level, 4);
    }

    #[test]
    fn update_housing_name_and_greeting_persist_for_estate() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        assert!(db.update_housing_name(estate.land_ident, "A Better Name"));
        assert!(
            db.update_housing_greeting(estate.land_ident, "Welcome to the local debug estate.")
        );

        let updated = db
            .housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
            .unwrap();
        assert_eq!(updated.estate_name, "A Better Name");
        assert_eq!(updated.greeting, "Welcome to the local debug estate.");
    }

    #[test]
    fn update_housing_name_clamps_to_packet_payload_bytes_on_utf8_boundary() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        assert!(db.update_housing_name(estate.land_ident, "abcdefghijklmnopqrs한글이잘립니다"));

        let updated = db
            .housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
            .unwrap();

        assert_eq!(updated.estate_name.as_bytes().len(), 19);
        assert_eq!(updated.estate_name, "abcdefghijklmnopqrs");
        assert!(std::str::from_utf8(updated.estate_name.as_bytes()).is_ok());
    }

    #[test]
    fn update_housing_greeting_clamps_to_packet_payload_bytes_on_utf8_boundary() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);
        let long_greeting = format!("{}한글이잘립니다", "a".repeat(191));

        assert!(db.update_housing_greeting(estate.land_ident, &long_greeting));

        let updated = db
            .housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
            .unwrap();

        assert_eq!(updated.greeting.as_bytes().len(), 191);
        assert_eq!(updated.greeting, "a".repeat(191));
        assert!(std::str::from_utf8(updated.greeting.as_bytes()).is_ok());
    }

    #[test]
    fn update_housing_fixture_json_persists_for_estate() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        assert!(db.update_housing_exterior_json(
            estate.land_ident,
            r#"{"roof_id":9,"colors":{"walls":5}}"#,
        ));
        assert!(db.update_housing_interior_json(
            estate.land_ident,
            r#"{"window_style":1,"ground_floor":65591}"#,
        ));

        let updated = db
            .housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
            .unwrap();

        assert_eq!(
            updated.exterior_json,
            r#"{"roof_id":9,"colors":{"walls":5}}"#
        );
        assert_eq!(
            updated.interior_json,
            r#"{"window_style":1,"ground_floor":65591}"#
        );
    }

    #[test]
    fn ensure_local_estate_default_name_is_clamped_to_packet_payload_bytes() {
        let mut db = test_db();

        let estate = db.ensure_local_estate(100, "abcdefghijklmnopqrs한글", 67);

        assert!(estate.estate_name.as_bytes().len() <= 20);
        assert!(std::str::from_utf8(estate.estate_name.as_bytes()).is_ok());
    }

    #[test]
    fn delete_housing_furniture_for_estate_keeps_estate() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 1000,
            catalog_id: 55,
            placed: true,
            ..Default::default()
        });

        assert_eq!(db.delete_housing_furniture_for_estate(estate.land_ident), 1);
        assert!(db.list_all_housing_furniture(estate.land_ident).is_empty());
        assert!(
            db.housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
                .is_some()
        );
    }

    #[test]
    fn replace_housing_placed_furniture_replaces_all_placed_rows_and_preserves_storerooms() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        for row in [
            HousingFurniture {
                land_ident: estate.land_ident,
                container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
                slot: 0,
                item_id: 1000,
                catalog_id: 55,
                placed: true,
                ..Default::default()
            },
            HousingFurniture {
                land_ident: estate.land_ident,
                container_type: container_type_to_i32(ContainerType::HousingInteriorStoreroom1),
                slot: 0,
                item_id: 1001,
                catalog_id: 56,
                placed: false,
                ..Default::default()
            },
            HousingFurniture {
                land_ident: estate.land_ident,
                container_type: container_type_to_i32(ContainerType::HousingExteriorPlacedItems),
                slot: 0,
                item_id: 1002,
                catalog_id: 57,
                placed: true,
                ..Default::default()
            },
            HousingFurniture {
                land_ident: estate.land_ident,
                container_type: container_type_to_i32(ContainerType::HousingExteriorStoreroom),
                slot: 0,
                item_id: 1003,
                catalog_id: 58,
                placed: false,
                ..Default::default()
            },
        ] {
            db.upsert_housing_furniture(row);
        }

        let deleted = db
            .replace_housing_placed_furniture_for_estate(
                estate.land_ident,
                true,
                true,
                &[
                    HousingFurniture {
                        land_ident: estate.land_ident,
                        container_type: container_type_to_i32(
                            ContainerType::HousingInteriorPlacedItems1,
                        ),
                        slot: 0,
                        item_id: 2000,
                        catalog_id: 155,
                        placed: true,
                        ..Default::default()
                    },
                    HousingFurniture {
                        land_ident: estate.land_ident,
                        container_type: container_type_to_i32(
                            ContainerType::HousingExteriorPlacedItems,
                        ),
                        slot: 0,
                        item_id: 2002,
                        catalog_id: 157,
                        placed: true,
                        ..Default::default()
                    },
                ],
            )
            .unwrap();

        let item_ids = db
            .list_all_housing_furniture(estate.land_ident)
            .into_iter()
            .map(|row| row.item_id)
            .collect::<Vec<_>>();

        assert_eq!(deleted, 2);
        assert!(item_ids.contains(&2000));
        assert!(item_ids.contains(&2002));
        assert!(item_ids.contains(&1001));
        assert!(item_ids.contains(&1003));
        assert!(!item_ids.contains(&1000));
        assert!(!item_ids.contains(&1002));
    }

    #[test]
    fn replace_housing_placed_furniture_can_target_only_interior_placed_rows() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 1000,
            catalog_id: 55,
            placed: true,
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingExteriorPlacedItems),
            slot: 0,
            item_id: 1002,
            catalog_id: 57,
            placed: true,
            ..Default::default()
        });

        let deleted = db
            .replace_housing_placed_furniture_for_estate(
                estate.land_ident,
                true,
                false,
                &[HousingFurniture {
                    land_ident: estate.land_ident,
                    container_type: container_type_to_i32(
                        ContainerType::HousingInteriorPlacedItems1,
                    ),
                    slot: 0,
                    item_id: 2000,
                    catalog_id: 155,
                    placed: true,
                    ..Default::default()
                }],
            )
            .unwrap();

        let item_ids = db
            .list_all_housing_furniture(estate.land_ident)
            .into_iter()
            .map(|row| row.item_id)
            .collect::<Vec<_>>();

        assert_eq!(deleted, 1);
        assert!(item_ids.contains(&2000));
        assert!(item_ids.contains(&1002));
        assert!(!item_ids.contains(&1000));
    }

    #[test]
    fn replace_housing_placed_furniture_handles_large_interior_presets() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);
        let mut rows = Vec::new();

        for slot in 0..600 {
            rows.push(HousingFurniture {
                land_ident: estate.land_ident,
                container_type: container_type_to_i32(interior_placed_containers()[slot / 50]),
                slot: (slot % 50) as i32,
                item_id: 1000 + slot as i64,
                catalog_id: slot as i32,
                placed: true,
                ..Default::default()
            });
        }

        db.replace_housing_placed_furniture_for_estate(estate.land_ident, true, false, &rows)
            .unwrap();

        let imported = db.list_housing_furniture(estate.land_ident, true);

        assert_eq!(imported.len(), 600);
        assert_eq!(imported.first().map(|row| row.slot), Some(0));
        assert_eq!(imported.last().map(|row| row.slot), Some(49));
        assert_eq!(
            imported.last().map(|row| row.container_type),
            Some(container_type_to_i32(
                ContainerType::HousingInteriorPlacedItems12
            ))
        );
    }

    #[test]
    fn delete_housing_estate_and_furniture_removes_both() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 1000,
            catalog_id: 55,
            placed: true,
            ..Default::default()
        });

        assert!(db.delete_housing_estate_and_furniture(estate.land_ident));
        assert!(db.list_all_housing_furniture(estate.land_ident).is_empty());
        assert!(
            db.housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
                .is_none()
        );
    }

    #[test]
    fn housing_estates_by_ward_filters_location_excludes_apartments_and_orders_plots() {
        let mut db = test_db();

        diesel::insert_into(housing_estates::table)
            .values(vec![
                HousingEstate {
                    land_ident: 1005,
                    house_id: 1005,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 0,
                    plot_index: 5,
                    owner_content_id: Some(100),
                    owner_name: "Plot Five".to_string(),
                    ..Default::default()
                },
                HousingEstate {
                    land_ident: 1002,
                    house_id: 1002,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 0,
                    plot_index: 2,
                    owner_content_id: Some(101),
                    owner_name: "Plot Two".to_string(),
                    ..Default::default()
                },
                HousingEstate {
                    land_ident: 2005,
                    house_id: 2005,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 68,
                    ward_index: 0,
                    division: 0,
                    plot_index: 5,
                    ..Default::default()
                },
                HousingEstate {
                    land_ident: 3005,
                    house_id: 3005,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 1,
                    plot_index: 5,
                    ..Default::default()
                },
                HousingEstate {
                    land_ident: 4005,
                    house_id: 4005,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 0,
                    plot_index: 5,
                    is_apartment: true,
                    ..Default::default()
                },
            ])
            .execute(&mut db.connection)
            .unwrap();

        let rows = db.housing_estates_by_ward(DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID, 67, 0, 0);

        assert_eq!(
            rows.iter()
                .map(|estate| estate.plot_index)
                .collect::<Vec<_>>(),
            vec![2, 5]
        );
    }

    #[test]
    fn housing_estates_by_ward_and_divisions_separates_and_orders_main_and_subdivision() {
        let mut db = test_db();

        diesel::insert_into(housing_estates::table)
            .values(vec![
                HousingEstate {
                    land_ident: 1105,
                    house_id: 1105,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 0,
                    plot_index: 5,
                    owner_name: "Main Five".to_string(),
                    ..Default::default()
                },
                HousingEstate {
                    land_ident: 1102,
                    house_id: 1102,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 0,
                    plot_index: 2,
                    owner_name: "Main Two".to_string(),
                    ..Default::default()
                },
                HousingEstate {
                    land_ident: 2104,
                    house_id: 2104,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 1,
                    plot_index: 4,
                    owner_name: "Subdivision Four".to_string(),
                    ..Default::default()
                },
                HousingEstate {
                    land_ident: 2101,
                    house_id: 2101,
                    territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID as i32,
                    world_id: 67,
                    ward_index: 0,
                    division: 1,
                    plot_index: 1,
                    owner_name: "Subdivision One".to_string(),
                    ..Default::default()
                },
            ])
            .execute(&mut db.connection)
            .unwrap();

        let (main, subdivision) = db.housing_estates_by_ward_and_divisions(
            DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            67,
            0,
        );

        assert_eq!(
            main.iter()
                .map(|estate| estate.plot_index)
                .collect::<Vec<_>>(),
            vec![2, 5]
        );
        assert_eq!(
            subdivision
                .iter()
                .map(|estate| estate.plot_index)
                .collect::<Vec<_>>(),
            vec![1, 4]
        );
    }

    #[test]
    fn ensure_local_apartment_creates_apartment_house_id() {
        let mut db = test_db();

        let apartment = db
            .ensure_local_apartment(
                100,
                "Tester",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                12,
            )
            .expect("room 12 apartment should be created");

        let house_id = HouseId::from_u64(apartment.house_id as u64);
        assert!(apartment.is_apartment);
        assert_eq!(apartment.room_number, 12);
        assert_eq!(apartment.land_ident, apartment.house_id);
        assert_eq!(house_id.room_number, 12);
        assert_eq!(
            house_id.territory_type_id,
            DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID
        );
        assert_eq!(house_id.world_id, 67);
        assert!(house_id.unit.apartment_flag);
        assert_eq!(
            house_id.unit.apartment_division_plot_index,
            DEFAULT_LOCAL_HOUSING_DIVISION
        );
    }

    #[test]
    fn ensure_local_apartment_rejects_room_number_overflow() {
        let mut db = test_db();

        let room_1 = db
            .ensure_local_apartment(
                100,
                "Room One",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                1,
            )
            .expect("room 1 apartment should be created");

        assert!(
            db.ensure_local_apartment(
                100,
                "Room 1025",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                1025,
            )
            .is_none(),
            "room 1025 must be rejected before it can alias room 1's packed HouseId"
        );

        let preserved = db
            .housing_estate_by_house_id(HouseId::from_u64(room_1.house_id as u64))
            .expect("room 1 house id should still resolve after rejecting room 1025");
        assert_eq!(preserved.room_number, 1);
        assert!(
            db.housing_apartment_by_room(
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                67,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                1025,
            )
            .is_none(),
            "out-of-range apartment rooms must not be queryable"
        );
    }

    #[test]
    fn ensure_local_apartment_does_not_delete_existing_house_or_house_furniture() {
        let mut db = test_db();
        let house = db.ensure_local_estate(100, "Tester", 67);
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: house.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 4000,
            catalog_id: 91,
            placed: true,
            ..Default::default()
        });

        let apartment = db
            .ensure_local_apartment(
                100,
                "Tester",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                7,
            )
            .expect("room 7 apartment should be created");

        let preserved_house = db
            .housing_estate_by_house_id(HouseId::from_u64(house.house_id as u64))
            .expect("house row should survive apartment grant");
        let house_furniture = db.list_housing_furniture(preserved_house.land_ident, true);

        assert_eq!(preserved_house.land_ident, house.land_ident);
        assert_eq!(house_furniture.len(), 1);
        assert_eq!(house_furniture[0].item_id, 4000);
        assert!(
            db.housing_estate_by_house_id(HouseId::from_u64(apartment.house_id as u64))
                .is_some(),
            "apartment row should be created without removing the house row"
        );
    }

    #[test]
    fn housing_apartments_by_ward_orders_rooms_and_room_lookup() {
        let mut db = test_db();

        let room_12 = db
            .ensure_local_apartment(
                100,
                "Room Twelve",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                12,
            )
            .expect("room 12 apartment should exist");
        let room_2 = db
            .ensure_local_apartment(
                101,
                "Room Two",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                2,
            )
            .expect("room 2 apartment should exist");
        let _other_ward = db
            .ensure_local_apartment(
                102,
                "Other Ward",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX + 1,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                1,
            )
            .expect("other ward apartment should exist");

        let apartments = db.housing_apartments_by_ward(
            DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            67,
            DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            DEFAULT_LOCAL_HOUSING_DIVISION,
        );

        assert_eq!(
            apartments
                .iter()
                .map(|estate| estate.room_number)
                .collect::<Vec<_>>(),
            vec![2, 12]
        );

        let room_lookup = db
            .housing_apartment_by_room(
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                67,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                DEFAULT_LOCAL_HOUSING_DIVISION,
                12,
            )
            .expect("room 12 apartment should exist");
        assert_eq!(room_lookup.land_ident, room_12.land_ident);
        assert_ne!(room_lookup.land_ident, room_2.land_ident);
    }

    #[test]
    fn apartment_queries_distinguish_main_and_subdivision_room_one() {
        let mut db = test_db();

        let main = db
            .ensure_local_apartment(
                100,
                "Main Room One",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                0,
                1,
            )
            .expect("main-division apartment should exist");
        let subdivision = db
            .ensure_local_apartment(
                100,
                "Subdivision Room One",
                67,
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                1,
                1,
            )
            .expect("subdivision apartment should exist");

        assert_ne!(main.land_ident, subdivision.land_ident);
        assert_ne!(main.house_id, subdivision.house_id);

        let main_house_id = HouseId::from_u64(main.house_id as u64);
        let subdivision_house_id = HouseId::from_u64(subdivision.house_id as u64);
        assert_eq!(main_house_id.unit.apartment_division_plot_index, 0);
        assert_eq!(subdivision_house_id.unit.apartment_division_plot_index, 1);

        let main_rows = db.housing_apartments_by_ward(
            DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            67,
            DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            0,
        );
        let subdivision_rows = db.housing_apartments_by_ward(
            DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            67,
            DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            1,
        );

        assert_eq!(
            main_rows
                .iter()
                .map(|estate| estate.house_id)
                .collect::<Vec<_>>(),
            vec![main.house_id]
        );
        assert_eq!(
            subdivision_rows
                .iter()
                .map(|estate| estate.house_id)
                .collect::<Vec<_>>(),
            vec![subdivision.house_id]
        );

        let main_lookup = db
            .housing_apartment_by_room(
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                67,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                0,
                1,
            )
            .expect("main room 1 should be addressable");
        let subdivision_lookup = db
            .housing_apartment_by_room(
                DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
                67,
                DEFAULT_LOCAL_HOUSING_WARD_INDEX,
                1,
                1,
            )
            .expect("subdivision room 1 should be addressable");

        assert_eq!(main_lookup.house_id, main.house_id);
        assert_eq!(subdivision_lookup.house_id, subdivision.house_id);
    }

    #[test]
    fn owned_estates_returns_character_estate() {
        let mut db = test_db();

        db.ensure_local_estate(100, "Tester", 67);

        assert_eq!(db.owned_housing_estates(100).len(), 1);
        assert!(db.owned_housing_estates(200).is_empty());
    }

    #[test]
    fn upsert_furniture_placement_replaces_same_slot() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 10,
            catalog_id: 20,
            placed: true,
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 11,
            catalog_id: 21,
            placed: true,
            ..Default::default()
        });

        let rows = db.list_housing_furniture(estate.land_ident, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].item_id, 11);
        assert_eq!(rows[0].catalog_id, 21);
    }

    #[test]
    fn update_furniture_position_changes_only_position_fields() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 10,
            catalog_id: 20,
            placed: true,
            ..Default::default()
        });

        assert!(db.update_housing_furniture_position(
            estate.land_ident,
            ContainerType::HousingInteriorPlacedItems1,
            0,
            Position(Vec3::new(1.0, 2.0, 3.0)),
            1.25,
        ));

        let rows = db.list_housing_furniture(estate.land_ident, true);
        assert_eq!(rows[0].item_id, 10);
        assert_eq!(rows[0].catalog_id, 20);
        assert_eq!(rows[0].pos_x, 1.0);
        assert_eq!(rows[0].pos_y, 2.0);
        assert_eq!(rows[0].pos_z, 3.0);
        assert_eq!(rows[0].rotation, 1.25);
    }

    #[test]
    fn update_outdoor_housing_furniture_position_uses_exterior_placed_container() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingExteriorPlacedItems),
            slot: 5,
            item_id: 7118,
            catalog_id: 44,
            placed: true,
            ..Default::default()
        });

        assert!(db.update_housing_furniture_position(
            estate.land_ident,
            ContainerType::HousingExteriorPlacedItems,
            5,
            Position(Vec3::new(10.0, 20.0, 30.0)),
            2.5,
        ));

        let rows = db.list_housing_furniture(estate.land_ident, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].container_type, 25001);
        assert_eq!(rows[0].slot, 5);
        assert_eq!(rows[0].pos_x, 10.0);
        assert_eq!(rows[0].pos_y, 20.0);
        assert_eq!(rows[0].pos_z, 30.0);
        assert_eq!(rows[0].rotation, 2.5);
    }

    #[test]
    fn move_placed_furniture_to_inventory_deletes_row() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 1000,
            catalog_id: 55,
            placed: true,
            ..Default::default()
        });

        assert!(db.move_housing_furniture_to_container(
            estate.land_ident,
            ContainerType::HousingInteriorPlacedItems1,
            0,
            None,
            None,
            false,
        ));

        assert!(
            db.list_housing_furniture(estate.land_ident, true)
                .is_empty()
        );
        assert!(
            db.list_housing_furniture(estate.land_ident, false)
                .is_empty()
        );
    }

    #[test]
    fn move_placed_furniture_to_storeroom_marks_unplaced() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 1000,
            catalog_id: 55,
            placed: true,
            ..Default::default()
        });

        assert!(db.move_housing_furniture_to_container(
            estate.land_ident,
            ContainerType::HousingInteriorPlacedItems1,
            0,
            Some(ContainerType::HousingInteriorStoreroom1),
            Some(1),
            false,
        ));

        assert!(
            db.list_housing_furniture(estate.land_ident, true)
                .is_empty()
        );
        let rows = db.list_housing_furniture(estate.land_ident, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].container_type,
            container_type_to_i32(ContainerType::HousingInteriorStoreroom1)
        );
        assert_eq!(rows[0].slot, 1);
        assert!(!rows[0].placed);
    }

    #[test]
    fn move_outdoor_housing_placed_furniture_to_storeroom_marks_unplaced() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingExteriorPlacedItems),
            slot: 5,
            item_id: 7118,
            catalog_id: 44,
            placed: true,
            pos_x: 1.0,
            pos_y: 2.0,
            pos_z: 3.0,
            rotation: 1.25,
            ..Default::default()
        });

        assert!(db.move_housing_furniture_to_container(
            estate.land_ident,
            ContainerType::HousingExteriorPlacedItems,
            5,
            Some(ContainerType::HousingExteriorStoreroom),
            Some(2),
            false,
        ));

        assert!(
            db.list_housing_furniture(estate.land_ident, true)
                .is_empty()
        );
        let rows = db.list_housing_furniture(estate.land_ident, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].container_type,
            container_type_to_i32(ContainerType::HousingExteriorStoreroom)
        );
        assert_eq!(rows[0].slot, 2);
        assert!(!rows[0].placed);
        assert_eq!(rows[0].item_id, 7118);
        assert_eq!(rows[0].catalog_id, 44);
        assert_eq!(rows[0].pos_x, 1.0);
        assert_eq!(rows[0].rotation, 1.25);
    }

    #[test]
    fn move_storeroom_furniture_to_inventory_deletes_row() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorStoreroom1),
            slot: 1,
            item_id: 1000,
            catalog_id: 55,
            placed: false,
            ..Default::default()
        });

        assert!(db.move_housing_furniture_to_container(
            estate.land_ident,
            ContainerType::HousingInteriorStoreroom1,
            1,
            None,
            None,
            false,
        ));

        assert!(
            db.list_housing_furniture(estate.land_ident, true)
                .is_empty()
        );
        assert!(
            db.list_housing_furniture(estate.land_ident, false)
                .is_empty()
        );
    }

    #[test]
    fn move_outdoor_housing_storeroom_furniture_to_inventory_deletes_row() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingExteriorStoreroom),
            slot: 2,
            item_id: 7118,
            catalog_id: 44,
            placed: false,
            ..Default::default()
        });

        assert!(db.move_housing_furniture_to_container(
            estate.land_ident,
            ContainerType::HousingExteriorStoreroom,
            2,
            None,
            None,
            false,
        ));

        assert!(
            db.list_housing_furniture(estate.land_ident, true)
                .is_empty()
        );
        assert!(
            db.list_housing_furniture(estate.land_ident, false)
                .is_empty()
        );
    }

    #[test]
    fn housing_summary_rows_for_admin_returns_compact_rows_with_furniture_counts() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 1000,
            catalog_id: 55,
            stain: 3,
            placed: true,
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorStoreroom1),
            slot: 1,
            item_id: 1001,
            catalog_id: 56,
            placed: false,
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingExteriorPlacedItems),
            slot: 2,
            item_id: 1002,
            catalog_id: 57,
            placed: true,
            ..Default::default()
        });

        let rows = db.housing_summary_rows_for_admin();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];

        assert_eq!(row.land_ident, estate.land_ident);
        assert_eq!(row.owner_name, "Tester");
        assert_eq!(row.kind, "personal_estate");
        assert_eq!(row.size, "large");
        assert_eq!(row.plot, "Ward 1 Plot 6");
        assert_eq!(row.flags, estate.flags);
        assert_eq!(row.furniture_counts.indoor_placed, 1);
        assert_eq!(row.furniture_counts.indoor_storeroom, 1);
        assert_eq!(row.furniture_counts.outdoor_placed, 1);
        assert_eq!(row.furniture_counts.outdoor_storeroom, 0);
        assert_eq!(row.furniture_counts.total, 3);
    }

    #[test]
    fn housing_summary_rows_for_admin_exclude_verbose_estate_fields() {
        let mut db = test_db();
        let base = db.ensure_local_estate(100, "Tester", 67);

        for idx in 0..10 {
            let mut estate = base.clone();
            estate.land_ident = base.land_ident + idx as i64 + 1;
            estate.house_id = estate.land_ident;
            estate.owner_content_id = Some(10_000 + idx as i64);
            estate.owner_name = format!("Owner-{idx:02}");
            estate.estate_name = format!("Estate-{idx:02}-{}", "y".repeat(96));
            estate.greeting = "g".repeat(220);
            estate.created_at += idx as i64 + 1;
            estate.updated_at += idx as i64 + 1;
            db.insert_housing_estate_for_test(estate);
        }

        let rows = db.housing_summary_rows_for_admin();
        assert_eq!(rows.len(), 11);
        assert!(rows.iter().all(|row| !row.owner_name.is_empty()));
        assert!(rows.iter().all(|row| row.furniture_counts.total == 0));
    }

    #[test]
    fn housing_estate_detail_for_admin_includes_counts_and_furniture_rows() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
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
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingExteriorStoreroom),
            slot: 1,
            item_id: 2001,
            catalog_id: 89,
            placed: false,
            ..Default::default()
        });

        let detail = db
            .housing_estate_detail_for_admin(estate.land_ident)
            .expect("detail should exist for local estate");

        assert_eq!(detail.estate.land_ident, estate.land_ident);
        assert_eq!(detail.estate.owner_name, "Tester");
        assert_eq!(detail.furniture_counts.indoor_placed, 1);
        assert_eq!(detail.furniture_counts.outdoor_storeroom, 1);
        assert_eq!(detail.furniture_counts.total, 2);
        assert_eq!(detail.furniture.len(), 2);
        assert_eq!(detail.furniture[0].item_id, 2000);
        assert_eq!(detail.furniture[0].container_kind, "indoor_placed");
        assert_eq!(detail.furniture[1].item_id, 2001);
        assert_eq!(detail.furniture[1].container_kind, "outdoor_storeroom");
    }

    #[test]
    fn housing_estate_detail_for_admin_returns_none_when_estate_missing() {
        let mut db = test_db();

        assert!(db.housing_estate_detail_for_admin(i64::MAX).is_none());
    }

    #[test]
    fn export_and_import_housing_estate_round_trips_estate_and_furniture() {
        let mut db = test_db();
        let estate = db.ensure_local_estate(100, "Tester", 67);

        assert!(db.update_housing_name(estate.land_ident, "Admin Export Estate"));
        assert!(
            db.update_housing_greeting(estate.land_ident, "Welcome back to the restored estate.",)
        );
        assert!(db.update_housing_exterior_json(
            estate.land_ident,
            r#"{"roof_id":9,"colors":{"walls":5}}"#,
        ));
        assert!(db.update_housing_interior_json(
            estate.land_ident,
            r#"{"ground_floor":65591,"lighting":3}"#,
        ));
        assert!(db.update_housing_light_level(estate.land_ident, 4));

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 3000,
            catalog_id: 91,
            stain: 7,
            placed: true,
            pos_x: 1.0,
            pos_y: 2.0,
            pos_z: 3.0,
            rotation: 1.5,
            created_by_content_id: Some(100),
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: estate.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorStoreroom1),
            slot: 1,
            item_id: 3001,
            catalog_id: 92,
            stain: 8,
            placed: false,
            created_by_content_id: Some(100),
            ..Default::default()
        });

        let export = db
            .export_housing_estate(estate.land_ident)
            .expect("export must include the estate");
        assert_eq!(export.estate.land_ident, estate.land_ident);
        assert_eq!(export.furniture.len(), 2);

        assert!(db.delete_housing_estate_and_furniture(estate.land_ident));
        assert!(db.import_housing_estate(export.clone()));

        let imported = db
            .housing_estate_by_house_id(HouseId::from_u64(estate.house_id as u64))
            .expect("estate must be restored after import");
        let furniture = db.list_all_housing_furniture(estate.land_ident);

        assert_eq!(imported.estate_name, "Admin Export Estate");
        assert_eq!(imported.greeting, "Welcome back to the restored estate.");
        assert_eq!(
            imported.exterior_json,
            r#"{"roof_id":9,"colors":{"walls":5}}"#
        );
        assert_eq!(
            imported.interior_json,
            r#"{"ground_floor":65591,"lighting":3}"#
        );
        assert_eq!(imported.light_level, 4);
        assert_eq!(furniture.len(), 2);
        assert_eq!(furniture[0].item_id, 3000);
        assert_eq!(furniture[1].item_id, 3001);
        assert_eq!(furniture[0].created_by_content_id, Some(100));
        assert_eq!(furniture[1].created_by_content_id, Some(100));
    }

    #[test]
    fn import_housing_estate_normalizes_furniture_land_ident_to_imported_estate() {
        let mut db = test_db();
        let target = db.ensure_local_estate(100, "Tester", 67);
        let foreign = db.ensure_local_estate_with_spec(HousingEstateSpec {
            owner_content_id: 200,
            owner_name: "Foreign".to_string(),
            world_id: 67,
            territory_type_id: DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID,
            ward_index: DEFAULT_LOCAL_HOUSING_WARD_INDEX,
            division: DEFAULT_LOCAL_HOUSING_DIVISION,
            plot_index: DEFAULT_LOCAL_HOUSING_PLOT_INDEX + 1,
            plot_size: PlotSize::Medium,
            free_company: false,
        });

        db.upsert_housing_furniture(HousingFurniture {
            land_ident: target.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorPlacedItems1),
            slot: 0,
            item_id: 4000,
            catalog_id: 120,
            placed: true,
            ..Default::default()
        });
        db.upsert_housing_furniture(HousingFurniture {
            land_ident: target.land_ident,
            container_type: container_type_to_i32(ContainerType::HousingInteriorStoreroom1),
            slot: 1,
            item_id: 4001,
            catalog_id: 121,
            placed: false,
            ..Default::default()
        });

        let mut export = db
            .export_housing_estate(target.land_ident)
            .expect("target estate should export");
        export.furniture[0].land_ident = foreign.land_ident;
        export.furniture[1].land_ident = i64::MAX;

        assert!(db.delete_housing_estate_and_furniture(target.land_ident));
        assert!(db.import_housing_estate(export));

        let imported_rows = db.list_all_housing_furniture(target.land_ident);
        assert_eq!(imported_rows.len(), 2);
        assert!(
            imported_rows
                .iter()
                .all(|row| row.land_ident == target.land_ident)
        );
        assert!(db.list_all_housing_furniture(foreign.land_ident).is_empty());
        assert!(db.list_all_housing_furniture(i64::MAX).is_empty());
    }
}
