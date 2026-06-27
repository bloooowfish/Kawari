use super::*;

impl ZoneConnection {
    pub async fn send_housing_outdoor_init(&mut self, zone_id: u16) {
        let active_context = outdoor_init_active_context(
            self.active_housing_ward_context,
            self.active_housing_estate_ward_context(),
        );
        let context = outdoor_init_display_context(
            active_context,
            self.display_housing_ward_context,
            self.default_housing_ward_context(),
            zone_id,
        );
        self.display_housing_ward_context = Some(context);
        let ward_index = context.ward_index;
        self.active_housing_ward_context =
            outdoor_init_authoritative_context(self.active_housing_ward_context, context, zone_id);
        let (main_estates, outdoor_furniture_lists, outdoor_furniture_objects) = {
            let mut database = self.database.lock();
            let (main_estates, subdivision_estates) = database
                .housing_estates_by_ward_and_divisions(zone_id, self.config.world_id, ward_index);
            let mut furniture_lists = Vec::new();
            let mut furniture_objects = Vec::new();

            for estate in main_estates.iter().chain(subdivision_estates.iter()) {
                let furniture_rows = database.list_housing_furniture(estate.land_ident, true);
                furniture_lists.extend(build_outdoor_estate_furniture_lists(
                    estate,
                    &furniture_rows,
                ));
                let scope = housing_furniture_object_scope_for_estate(estate, false);
                furniture_objects.extend(furniture_rows.iter().filter_map(|row| {
                    housing_furniture_object_from_row(
                        row,
                        false,
                        scope.ward_index,
                        scope.plot_index,
                    )
                }));
            }

            (main_estates, furniture_lists, furniture_objects)
        };
        let houses = build_house_list_houses(&main_estates);

        self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::HouseList(
            HouseList {
                land_id: 0,
                ward: ward_index as u16,
                territory_type_id: zone_id,
                world_id: self.config.world_id,
                subdivision: 257,
                houses,
            },
        )))
        .await;

        if outdoor_furniture_lists.is_empty() {
            for index in 0..8 {
                self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                    FurnitureList {
                        count: 8,
                        index,
                        ..Default::default()
                    },
                )))
                .await;
            }
        } else {
            for list in outdoor_furniture_lists {
                self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                    list,
                )))
                .await;
            }
        }

        self.handle
            .send(ToServer::SyncHousingFurnitureObjects(
                self.player_data.character.actor_id,
                outdoor_furniture_objects,
            ))
            .await;
    }

    pub async fn send_housing_indoor_init(&mut self, zone_id: u16) {
        if housing_indoor_init_needs_resolution(self.active_housing_estate.as_ref()) {
            self.resolve_active_housing_estate(TerritoryIntendedUse::HousingIndoor, zone_id);
        }

        let Some(active_estate) = self.active_housing_estate.clone() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                zone_id,
                "Unable to resolve active housing estate for indoor init"
            );

            self.pending_housing_indoor_furniture_object_overlay_sync = false;
            self.send_housing_interior_details("", 0, false).await;
            self.send_housing_furniture_lists(HouseId::default(), &[], true, None)
                .await;
            return;
        };

        if !active_estate.indoors {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                "Active housing estate was resolved as outdoor during indoor init"
            );
        }

        let (
            interior_json,
            light_level,
            is_apartment,
            hide_chambers_door,
            furniture_rows,
            all_furniture_rows,
            estate,
        ) = {
            let mut database = self.database.lock();
            let estate = database.housing_estate_by_house_id(active_estate.house_id);
            let interior_json = estate
                .as_ref()
                .map(|estate| estate.interior_json.clone())
                .unwrap_or_default();
            let light_level = estate
                .as_ref()
                .map(|estate| estate.light_level.clamp(0, u8::MAX as i32) as u8)
                .unwrap_or_default();
            let is_apartment = estate.as_ref().is_some_and(|estate| estate.is_apartment);
            let hide_chambers_door = estate
                .as_ref()
                .map(|estate| should_hide_additional_chambers_door(estate.flags))
                .unwrap_or(true);
            let all_furniture_rows = database.list_all_housing_furniture(active_estate.land_ident);
            let furniture_rows = all_furniture_rows
                .iter()
                .filter(|row| is_interior_placed_furniture_row(row))
                .cloned()
                .collect::<Vec<_>>();

            (
                interior_json,
                light_level,
                is_apartment,
                hide_chambers_door,
                furniture_rows,
                all_furniture_rows,
                estate,
            )
        };
        let indoor_slot_capacity = estate
            .as_ref()
            .map(housing_interior_placed_slot_capacity_for_estate);
        let mut house_inventory = housing_inventory_from_rows(&all_furniture_rows);
        if let Some(estate) = estate.as_ref() {
            let mut game_data = self.gamedata.lock();
            populate_housing_appearance_inventory(&mut house_inventory, &mut game_data, estate);
        }
        self.player_data.house_inventory = house_inventory;

        tracing::warn!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            house_id = active_estate.house_id.to_u64(),
            furniture_count = furniture_rows.len(),
            "Sending housing indoor init"
        );

        self.send_housing_interior_details(&interior_json, light_level, is_apartment)
            .await;
        let lists = build_furniture_lists(
            active_estate.house_id,
            &furniture_rows,
            true,
            indoor_slot_capacity,
        );
        let initial_list_count = initial_housing_furniture_list_count(true, lists.len());
        self.pending_housing_indoor_furniture_list_tail = initial_list_count < lists.len();
        self.pending_housing_indoor_finish_loading = false;
        self.pending_housing_indoor_furniture_object_overlay_sync = true;
        self.send_built_housing_furniture_lists(
            lists,
            active_estate.house_id,
            furniture_rows.len(),
            true,
            indoor_slot_capacity,
            0,
            initial_list_count,
            if self.pending_housing_indoor_furniture_list_tail {
                "before_finish_loading"
            } else {
                "all"
            },
        )
        .await;
        if should_finish_housing_object_data_after_initial_indoor_lists(
            TerritoryIntendedUse::HousingIndoor,
            self.pending_housing_indoor_furniture_list_tail,
        ) {
            self.send_housing_object_data_value_sets(&furniture_rows)
                .await;
            self.send_housing_interior_ready(active_estate.house_id)
                .await;
        }
        if hide_chambers_door {
            self.actor_control_self(ActorControlCategory::HideAdditionalChambersDoor {})
                .await;
        }
    }

    pub async fn send_deferred_indoor_furniture_after_remodel(&mut self) -> bool {
        if !self.pending_housing_indoor_furniture_list_tail {
            return false;
        }
        self.pending_housing_indoor_furniture_list_tail = false;

        if housing_indoor_init_needs_resolution(self.active_housing_estate.as_ref()) {
            self.resolve_active_housing_estate(
                TerritoryIntendedUse::HousingIndoor,
                self.player_data.volatile.zone_id as u16,
            );
        }

        let Some(active_estate) = self.active_housing_estate.clone() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                zone_id = self.player_data.volatile.zone_id,
                "Unable to resolve active housing estate for deferred furniture list tail"
            );
            return false;
        };

        let (furniture_rows, slot_capacity) = {
            let mut database = self.database.lock();
            let estate = database.housing_estate_by_house_id(active_estate.house_id);
            let furniture_rows = database
                .list_all_housing_furniture(active_estate.land_ident)
                .into_iter()
                .filter(is_interior_placed_furniture_row)
                .collect::<Vec<_>>();
            (
                furniture_rows,
                estate
                    .as_ref()
                    .map(housing_interior_placed_slot_capacity_for_estate),
            )
        };

        tracing::warn!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            house_id = active_estate.house_id.to_u64(),
            furniture_count = furniture_rows.len(),
            "Sending deferred indoor housing furniture list tail after remodel gate"
        );

        self.send_deferred_housing_furniture_lists(
            active_estate.house_id,
            &furniture_rows,
            true,
            slot_capacity,
        )
        .await;
        self.send_housing_object_data_value_sets(&furniture_rows)
            .await;
        self.send_housing_interior_ready(active_estate.house_id)
            .await;
        true
    }

    async fn send_housing_object_data_value_sets(&mut self, rows: &[HousingFurniture]) {
        let value_sets = housing_object_data_value_sets_from_rows(rows);
        tracing::debug!(
            content_id = self.player_data.character.content_id,
            value_set_count = value_sets.len(),
            "Sending housing object data value sets"
        );

        for value_set in value_sets {
            self.send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingObjectDataValueSet(value_set),
            ))
            .await;
        }
    }

    async fn send_housing_interior_ready(&mut self, house_id: HouseId) {
        let primary_id = housing_interior_ready_primary_id(house_id);
        let secondary_id = housing_interior_ready_secondary_id(self.active_housing_estate.as_ref());
        tracing::debug!(
            content_id = self.player_data.character.content_id,
            house_id = house_id.to_u64(),
            primary_id,
            secondary_id,
            "Sending housing interior ready actor control"
        );
        self.actor_control_self(ActorControlCategory::HousingInteriorReady {
            primary_id,
            secondary_id,
            unk1: 0,
        })
        .await;
    }

    pub async fn sync_deferred_indoor_overlays_after_load(
        &mut self,
        intended_use: TerritoryIntendedUse,
        reason: &'static str,
    ) {
        if !should_sync_indoor_overlays_after_loading(
            intended_use,
            self.pending_housing_indoor_furniture_list_tail,
            self.pending_housing_indoor_furniture_object_overlay_sync,
        ) {
            return;
        }
        self.sync_pending_housing_indoor_furniture_object_overlays(reason)
            .await;
    }

    pub async fn sync_deferred_indoor_overlays_after_remodel(
        &mut self,
        intended_use: TerritoryIntendedUse,
        tail_sent: bool,
    ) {
        if !should_sync_indoor_overlays_after_remodel(
            intended_use,
            tail_sent,
            self.pending_housing_indoor_furniture_object_overlay_sync,
        ) {
            return;
        }
        self.sync_pending_housing_indoor_furniture_object_overlays("after_remodel_gate")
            .await;
    }

    pub async fn sync_housing_furniture_object_overlays_after_zone_in(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) {
        if !should_sync_indoor_overlays_on_finish_zoning(
            intended_use,
            self.pending_housing_indoor_furniture_object_overlay_sync,
        ) {
            return;
        }
        self.sync_pending_housing_indoor_furniture_object_overlays("finish_zoning")
            .await;
    }

    pub fn should_defer_housing_indoor_finish_loading(
        &self,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        should_defer_housing_indoor_finish_loading(
            intended_use,
            self.pending_housing_indoor_furniture_list_tail,
        )
    }

    async fn sync_pending_housing_indoor_furniture_object_overlays(
        &mut self,
        reason: &'static str,
    ) {
        if housing_indoor_init_needs_resolution(self.active_housing_estate.as_ref()) {
            self.resolve_active_housing_estate(
                TerritoryIntendedUse::HousingIndoor,
                self.player_data.volatile.zone_id as u16,
            );
        }

        let Some(active_estate) = self.active_housing_estate.clone() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                zone_id = self.player_data.volatile.zone_id,
                "Unable to resolve active housing estate for deferred furniture object overlay sync"
            );
            return;
        };

        self.pending_housing_indoor_furniture_object_overlay_sync = false;
        let furniture_objects = {
            let mut database = self.database.lock();
            housing_furniture_objects_from_rows(
                &database.list_all_housing_furniture(active_estate.land_ident),
                true,
                0,
                0,
            )
        };

        tracing::warn!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            house_id = active_estate.house_id.to_u64(),
            object_count = furniture_objects.len(),
            reason,
            "Syncing indoor housing furniture object overlays"
        );
        self.handle
            .send(ToServer::SyncHousingFurnitureObjects(
                self.player_data.character.actor_id,
                furniture_objects,
            ))
            .await;
    }
}

pub(super) fn housing_indoor_init_needs_resolution(
    active_estate: Option<&ActiveHousingEstate>,
) -> bool {
    !active_estate.is_some_and(|estate| estate.indoors)
}

fn housing_interior_placed_slot_capacity_for_estate(estate: &HousingEstate) -> usize {
    if estate.is_apartment {
        return Furniture::COUNT;
    }

    match housing_estate_plot_size(estate) {
        PlotSize::Small => 300,
        PlotSize::Medium => 450,
        PlotSize::Large => 600,
    }
}

pub(super) fn should_defer_housing_indoor_finish_loading(
    intended_use: TerritoryIntendedUse,
    pending_tail: bool,
) -> bool {
    intended_use == TerritoryIntendedUse::HousingIndoor && pending_tail
}

pub(super) fn should_finish_housing_object_data_after_initial_indoor_lists(
    intended_use: TerritoryIntendedUse,
    pending_tail: bool,
) -> bool {
    intended_use == TerritoryIntendedUse::HousingIndoor && !pending_tail
}

pub(super) fn housing_interior_ready_primary_id(house_id: HouseId) -> u64 {
    let value = house_id.to_u64();
    value.rotate_right(32)
}

fn housing_interior_ready_secondary_id(active_estate: Option<&ActiveHousingEstate>) -> u64 {
    const DEFAULT_HOUSING_READY_COOKIE: u32 = 0x0185_813a;
    const HOUSING_READY_MODE: u32 = 12;

    let cookie = active_estate
        .map(|estate| {
            stable_housing_ready_cookie(estate.land_ident as u64, estate.house_id.to_u64())
        })
        .unwrap_or(DEFAULT_HOUSING_READY_COOKIE);
    ((cookie as u64) << 32) | HOUSING_READY_MODE as u64
}

fn stable_housing_ready_cookie(land_ident: u64, house_id: u64) -> u32 {
    let mut value = 0x811c_9dc5u32;
    for byte in land_ident
        .to_le_bytes()
        .into_iter()
        .chain(house_id.to_le_bytes())
    {
        value ^= byte as u32;
        value = value.wrapping_mul(0x0100_0193);
    }
    value.max(1)
}

pub(super) fn housing_object_data_value_sets_from_rows(
    rows: &[HousingFurniture],
) -> Vec<HousingObjectDataValueSet> {
    rows.iter()
        .filter(|row| is_interior_placed_furniture_row(row))
        .filter_map(housing_object_data_value_set_from_row)
        .collect()
}

fn housing_object_data_value_set_from_row(
    row: &HousingFurniture,
) -> Option<HousingObjectDataValueSet> {
    let container = housing_container_type_from_i32(row.container_type)?;
    let furniture_index = flat_slot_for_container(container, row.slot as u16)?;
    Some(HousingObjectDataValueSet {
        furniture_index,
        ..Default::default()
    })
}

pub(super) fn should_sync_indoor_overlays_after_remodel(
    intended_use: TerritoryIntendedUse,
    tail_sent: bool,
    pending_overlay_sync: bool,
) -> bool {
    intended_use == TerritoryIntendedUse::HousingIndoor && tail_sent && pending_overlay_sync
}

pub(super) fn should_sync_indoor_overlays_after_loading(
    intended_use: TerritoryIntendedUse,
    pending_tail: bool,
    pending_overlay_sync: bool,
) -> bool {
    intended_use == TerritoryIntendedUse::HousingIndoor && !pending_tail && pending_overlay_sync
}

pub(super) fn should_sync_indoor_overlays_on_finish_zoning(
    _intended_use: TerritoryIntendedUse,
    _pending_overlay_sync: bool,
) -> bool {
    false
}

pub(super) fn build_outdoor_estate_furniture_lists(
    estate: &HousingEstate,
    rows: &[HousingFurniture],
) -> Vec<FurnitureList> {
    let exterior_rows = rows
        .iter()
        .filter(|row| is_exterior_placed_furniture_row(row))
        .cloned()
        .collect::<Vec<_>>();

    build_estate_furniture_lists(estate, &exterior_rows, false)
}

pub(super) fn housing_furniture_object_from_row(
    row: &HousingFurniture,
    indoors: bool,
    ward_index: u8,
    plot_index: u8,
) -> Option<HousingFurnitureObject> {
    if !row.placed {
        return None;
    }

    let container = housing_container_type_from_i32(row.container_type)?;
    if !housing_placed_container_matches_area(container, indoors) {
        return None;
    }

    let slot = flat_slot_for_container(container, u16::try_from(row.slot).ok()?)?;

    Some(HousingFurnitureObject {
        slot,
        catalog_id: row.catalog_id.clamp(0, u16::MAX as i32) as u16,
        position: Position(Vec3::new(row.pos_x, row.pos_y, row.pos_z)),
        rotation: row.rotation,
        indoors,
        ward_index,
        plot_index,
    })
}

pub(super) fn housing_furniture_objects_from_rows(
    rows: &[HousingFurniture],
    indoors: bool,
    ward_index: u8,
    plot_index: u8,
) -> Vec<HousingFurnitureObject> {
    rows.iter()
        .filter_map(|row| housing_furniture_object_from_row(row, indoors, ward_index, plot_index))
        .collect()
}

fn housing_placed_container_matches_area(container: ContainerType, indoors: bool) -> bool {
    if indoors {
        is_interior_placed_container(container)
    } else {
        container == ContainerType::HousingExteriorPlacedItems
    }
}

pub(super) fn is_interior_placed_furniture_row(row: &HousingFurniture) -> bool {
    if !row.placed {
        return false;
    }

    housing_container_type_from_i32(row.container_type).is_some_and(is_interior_placed_container)
}

fn is_exterior_placed_furniture_row(row: &HousingFurniture) -> bool {
    row.placed && row.container_type == ContainerType::HousingExteriorPlacedItems as u16 as i32
}

pub(super) fn should_hide_additional_chambers_door(flags: i32) -> bool {
    flags & FREE_COMPANY_HOUSING_FLAG == 0
}
