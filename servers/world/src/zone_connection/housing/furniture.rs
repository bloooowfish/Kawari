use super::*;

pub struct PersistedFurniturePlacement {
    pub container: ContainerType,
    pub slot: u16,
    pub catalog_id: u16,
    pub stain: u8,
    pub position: Position,
    pub indoors: bool,
    pub rotation: f32,
    pub ward_index: u8,
    pub plot_index: u8,
    pub object_slot: u16,
    pub spawned: bool,
}

pub struct PersistedFurnitureTranslation {
    pub storage_id: ContainerType,
    pub container_slot: u16,
    pub plot_number: u16,
    pub object_key: HousingFurnitureObjectKey,
}

pub struct PersistedHousingItemMove {
    pub item_id: u32,
    pub src_container: ContainerType,
    pub dst_container: ContainerType,
    pub ack_slot: u16,
    pub removed_from_world: bool,
    pub object_key: Option<HousingFurnitureObjectKey>,
}

impl ZoneConnection {
    pub fn reload_active_housing_inventory(&mut self, intended_use: TerritoryIntendedUse) {
        if self.active_housing_estate.is_none() {
            self.resolve_active_housing_estate(
                intended_use,
                self.player_data.volatile.zone_id as u16,
            );
        }

        let Some(active_estate) = self.active_housing_estate.as_ref() else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Unable to reload housing inventory without an active housing estate"
            );
            return;
        };

        let (rows, estate) = {
            let mut database = self.database.lock();
            (
                database.list_all_housing_furniture(active_estate.land_ident),
                database.housing_estate_by_house_id(active_estate.house_id),
            )
        };
        let mut house_inventory = housing_inventory_from_rows(&rows);
        if let Some(estate) = estate.as_ref() {
            let mut game_data = self.gamedata.lock();
            populate_housing_appearance_inventory(&mut house_inventory, &mut game_data, estate);
        }
        self.player_data.house_inventory = house_inventory;
    }

    pub async fn process_housing_appearance_item_operation(
        &mut self,
        action: &ItemOperation,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        if !housing_appearance_operation_involves_staging(action) {
            return false;
        }

        if !self.in_housing_area(intended_use)
            || !self.can_edit_active_housing_estate(intended_use)
            || !housing_appearance_operation_matches_area(action, intended_use)
        {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                src_storage = ?action.src_storage_id,
                dst_storage = ?action.dst_storage_id,
                "Rejecting housing appearance item operation outside the matching editable estate"
            );
            return true;
        }

        if !matches!(
            action.operation_type,
            ItemOperationKind::Move | ItemOperationKind::Exchange
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                operation_type = ?action.operation_type,
                "Ignoring unsupported housing appearance item operation"
            );
            return true;
        }

        let Some(src_item) =
            self.get_player_or_housing_item(action.src_storage_id, action.src_container_index)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                src_storage = ?action.src_storage_id,
                src_slot = action.src_container_index,
                "Housing appearance item operation referenced an invalid source slot"
            );
            return true;
        };

        let Some(dst_item) =
            self.get_player_or_housing_item(action.dst_storage_id, action.dst_container_index)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                dst_storage = ?action.dst_storage_id,
                dst_slot = action.dst_container_index,
                "Housing appearance item operation referenced an invalid destination slot"
            );
            return true;
        };

        if !self.set_player_or_housing_item(
            action.dst_storage_id,
            action.dst_container_index,
            src_item,
        ) || !self.set_player_or_housing_item(
            action.src_storage_id,
            action.src_container_index,
            dst_item,
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                src_storage = ?action.src_storage_id,
                dst_storage = ?action.dst_storage_id,
                "Housing appearance item operation failed while mutating local inventory state"
            );
            return true;
        }

        {
            let mut database = self.database.lock();
            database.commit_classjob_and_inventory(&self.player_data);
        }

        self.send_affected_containers(action.src_storage_id, action.dst_storage_id)
            .await;

        tracing::info!(
            content_id = self.player_data.character.content_id,
            src_storage = ?action.src_storage_id,
            src_slot = action.src_container_index,
            dst_storage = ?action.dst_storage_id,
            dst_slot = action.dst_container_index,
            "Processed housing appearance item operation"
        );

        true
    }

    pub fn record_housing_appearance_item_operation_marker(
        &mut self,
        action: &HousingItemOperation,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        let Some(operation_hint) = housing_item_operation_hint(action) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                raw = ?action.raw,
                "Ignoring housing appearance item operation without a usable source container hint"
            );
            return true;
        };
        let source_container = operation_hint.source_container;
        let source_slot = operation_hint.source_slot;

        if !self.in_housing_area(intended_use) || !self.can_edit_active_housing_estate(intended_use)
        {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Rejecting housing appearance item operation marker outside an editable estate"
            );
            return true;
        }

        let Some(source_item) = self
            .player_data
            .inventory
            .get_item(source_container, source_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?source_container,
                source_slot,
                "Housing appearance item operation marker referenced an invalid player inventory slot"
            );
            return true;
        };
        if source_item.is_empty_slot() {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?source_container,
                source_slot,
                "Housing appearance item operation marker referenced an empty player inventory slot"
            );
            return true;
        }

        let Some((_, item_ui_category)) = ({
            let mut gamedata = self.gamedata.lock();
            housing_appearance_item_data(&mut gamedata, source_item)
        }) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = source_item.item_id,
                "Housing appearance item operation marker source item is not a usable appearance fixture"
            );
            return true;
        };

        let marker_target_slot = Some(operation_hint.target_appearance_slot);
        let Some((target_container, target_slot)) =
            self.housing_appearance_target(item_ui_category, intended_use, marker_target_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = source_item.item_id,
                item_ui_category,
                intended_use = intended_use as u8,
                ?marker_target_slot,
                "Housing appearance item operation marker has no supported target slot"
            );
            return true;
        };

        self.pending_housing_appearance_item_operation =
            Some(PendingHousingAppearanceItemOperation {
                source_container,
                source_slot,
                target_container,
                target_slot,
            });

        tracing::info!(
            content_id = self.player_data.character.content_id,
            ?source_container,
            source_slot,
            item_id = source_item.item_id,
            item_ui_category,
            ?marker_target_slot,
            ?target_container,
            target_slot,
            "Staged housing appearance item operation marker"
        );

        true
    }

    pub fn clear_pending_housing_appearance_item_operation(&mut self) {
        self.pending_housing_appearance_item_operation = None;
    }

    pub fn apply_pending_housing_appearance_item_operation(
        &mut self,
        intended_use: TerritoryIntendedUse,
    ) -> Option<AppliedHousingAppearanceItemOperation> {
        let Some(operation) = self.pending_housing_appearance_item_operation.take() else {
            return None;
        };

        if housing_appearance_container_intended_use(operation.target_container)
            != Some(intended_use)
            || !self.in_housing_area(intended_use)
            || !self.can_edit_active_housing_estate(intended_use)
        {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                ?operation,
                "Rejecting staged housing appearance item operation outside its editable estate"
            );
            return None;
        }

        let Some(original_source_item) = self
            .player_data
            .inventory
            .get_item(operation.source_container, operation.source_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Staged housing appearance item operation lost its source item"
            );
            return None;
        };
        let Some(original_target_item) = self
            .player_data
            .house_inventory
            .get_item(operation.target_container, operation.target_slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Staged housing appearance item operation lost its target slot"
            );
            return None;
        };

        if original_source_item.is_empty_slot() {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Staged housing appearance item operation source item is empty at apply time"
            );
            return None;
        }

        if !self.housing_appearance_item_matches_target_slot(
            original_source_item,
            operation.target_container,
            operation.target_slot,
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = original_source_item.item_id,
                ?operation,
                "Staged housing appearance item operation source item does not match the target slot category"
            );
            return None;
        }

        if !self.set_player_or_housing_item(
            operation.source_container,
            operation.source_slot,
            original_target_item,
        ) || !self.set_player_or_housing_item(
            operation.target_container,
            operation.target_slot,
            original_source_item,
        ) {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?operation,
                "Failed to apply staged housing appearance item operation"
            );
            return None;
        }

        tracing::info!(
            content_id = self.player_data.character.content_id,
            ?operation,
            source_item_id = original_source_item.item_id,
            target_item_id = original_target_item.item_id,
            "Applied staged housing appearance item operation"
        );

        Some(AppliedHousingAppearanceItemOperation {
            source_container: operation.source_container,
            source_slot: operation.source_slot,
            target_container: operation.target_container,
            target_slot: operation.target_slot,
            original_source_item,
            original_target_item,
        })
    }

    pub fn rollback_housing_appearance_item_operation(
        &mut self,
        operation: AppliedHousingAppearanceItemOperation,
    ) {
        let _ = self.set_player_or_housing_item(
            operation.source_container,
            operation.source_slot,
            operation.original_source_item,
        );
        let _ = self.set_player_or_housing_item(
            operation.target_container,
            operation.target_slot,
            operation.original_target_item,
        );
    }

    pub async fn send_housing_appearance_item_operation_update(
        &mut self,
        operation: AppliedHousingAppearanceItemOperation,
        intended_use: TerritoryIntendedUse,
    ) {
        let source_update =
            ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
                sequence: self.player_data.item_sequence,
                container: operation.source_container,
                slot: operation.source_slot,
                ..operation.original_target_item.into()
            }));
        self.send_ipc_self(source_update).await;
        self.player_data.item_sequence += 1;

        let target_update =
            ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
                sequence: self.player_data.item_sequence,
                container: operation.target_container,
                slot: operation.target_slot,
                ..operation.original_source_item.into()
            }));
        self.send_ipc_self(target_update).await;
        self.player_data.item_sequence += 1;

        self.actor_control_self(ActorControlCategory::FinishEstateAppearanceItemOperation {})
            .await;

        if intended_use == TerritoryIntendedUse::HousingIndoor {
            self.send_current_housing_interior_details().await;
        }
    }

    pub fn persist_housing_appearance_remodel(
        &mut self,
        apply: bool,
        intended_use: TerritoryIntendedUse,
    ) -> bool {
        if !apply {
            self.reload_active_housing_inventory(intended_use);
            tracing::info!(
                content_id = self.player_data.character.content_id,
                intended_use = intended_use as u8,
                "Canceled housing appearance remodel"
            );
            return true;
        }

        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_owner_gate()
        } else {
            self.active_housing_estate_for_edit(intended_use)
        };
        let Some(active_estate) = active_estate else {
            return false;
        };

        let Some(estate) = ({
            let mut database = self.database.lock();
            database.housing_estate_by_house_id(active_estate.house_id)
        }) else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                house_id = active_estate.house_id.to_u64(),
                "Rejecting housing appearance remodel for an unknown estate"
            );
            return false;
        };

        let updated = match intended_use {
            TerritoryIntendedUse::HousingIndoor => {
                let Some(interior_json) = self.interior_json_from_current_appearance(&estate)
                else {
                    return false;
                };
                let mut database = self.database.lock();
                database.update_housing_interior_json(active_estate.land_ident, &interior_json)
            }
            TerritoryIntendedUse::HousingOutdoor => {
                let Some(exterior_json) = self.exterior_json_from_current_appearance(&estate)
                else {
                    return false;
                };
                let mut database = self.database.lock();
                database.update_housing_exterior_json(active_estate.land_ident, &exterior_json)
            }
            _ => false,
        };

        if updated {
            self.reload_active_housing_inventory(intended_use);
            tracing::info!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                intended_use = intended_use as u8,
                "Persisted housing appearance remodel"
            );
        } else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                intended_use = intended_use as u8,
                "Housing appearance remodel did not match a persisted estate"
            );
        }

        updated
    }

    fn interior_json_from_current_appearance(&mut self, estate: &HousingEstate) -> Option<String> {
        let items = self.housing_appearance_items(ContainerType::HousingInteriorAppearance);
        let mut json = estate.interior_json.clone();
        let mut gamedata = self.gamedata.lock();

        for (slot, item) in items {
            let Some(field) = housing_interior_field_for_appearance_slot(slot) else {
                continue;
            };
            let Some((additional_data, item_ui_category)) =
                housing_appearance_item_data(&mut gamedata, item)
            else {
                continue;
            };
            let expected_category = housing_interior_item_ui_category_for_slot(slot);
            if Some(item_ui_category) != expected_category {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    slot,
                    item_id = item.item_id,
                    item_ui_category,
                    ?expected_category,
                    "Skipping interior appearance item with an unexpected UI category"
                );
                continue;
            }

            json = match update_interior_json_field(&json, field, additional_data) {
                Ok(json) => json,
                Err(_) => return None,
            };
        }

        Some(json)
    }

    fn exterior_json_from_current_appearance(&mut self, estate: &HousingEstate) -> Option<String> {
        let items = self.housing_appearance_items(ContainerType::HousingExteriorAppearance);
        let mut json = estate.exterior_json.clone();
        let mut gamedata = self.gamedata.lock();

        for (slot, item) in items {
            let Some(field) = housing_exterior_field_for_appearance_slot(slot) else {
                continue;
            };
            let Some((additional_data, item_ui_category)) =
                housing_appearance_item_data(&mut gamedata, item)
            else {
                continue;
            };
            let expected_category = housing_exterior_item_ui_category_for_slot(slot);
            if Some(item_ui_category) != expected_category {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    slot,
                    item_id = item.item_id,
                    item_ui_category,
                    ?expected_category,
                    "Skipping exterior appearance item with an unexpected UI category"
                );
                continue;
            }

            json = match update_exterior_json_field(&json, field, additional_data as u16) {
                Ok(json) => json,
                Err(_) => return None,
            };
            if item.stains[0] != 0 {
                let Some(color_field) = housing_exterior_color_field_for_appearance_slot(slot)
                else {
                    continue;
                };
                json = match update_exterior_json_color(&json, color_field, item.stains[0]) {
                    Ok(json) => json,
                    Err(_) => return None,
                };
            }
        }

        Some(json)
    }

    fn housing_appearance_items(&self, container: ContainerType) -> Vec<(u16, Item)> {
        let Some(container) = self.player_data.house_inventory.get_container(container) else {
            return Vec::new();
        };

        (0..container.max_slots())
            .filter_map(|slot| {
                let item = *container.get_slot(slot as u16);
                (!item.is_empty_slot()).then_some((slot as u16, item))
            })
            .collect()
    }

    fn housing_appearance_target(
        &mut self,
        item_ui_category: u8,
        intended_use: TerritoryIntendedUse,
        marker_target_slot: Option<u16>,
    ) -> Option<(ContainerType, u16)> {
        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_owner_gate()
        } else {
            self.active_housing_estate_for_edit(intended_use)
        }?;

        let is_apartment = if intended_use == TerritoryIntendedUse::HousingIndoor {
            let mut database = self.database.lock();
            database
                .housing_estate_by_house_id(active_estate.house_id)
                .is_some_and(|estate| estate.is_apartment)
        } else {
            false
        };

        let target_container = match intended_use {
            TerritoryIntendedUse::HousingIndoor => ContainerType::HousingInteriorAppearance,
            TerritoryIntendedUse::HousingOutdoor => ContainerType::HousingExteriorAppearance,
            _ => return None,
        };
        let target_slot = housing_appearance_target_slot(
            item_ui_category,
            intended_use,
            is_apartment,
            marker_target_slot,
        )?;

        Some((target_container, target_slot))
    }

    fn housing_appearance_item_matches_target_slot(
        &mut self,
        item: Item,
        target_container: ContainerType,
        target_slot: u16,
    ) -> bool {
        let expected_category = match target_container {
            ContainerType::HousingInteriorAppearance
            | ContainerType::HousingInteriorAppearanceEdit => {
                housing_interior_item_ui_category_for_slot(target_slot)
            }
            ContainerType::HousingExteriorAppearance
            | ContainerType::HousingExteriorAppearanceEdit => {
                housing_exterior_item_ui_category_for_slot(target_slot)
            }
            _ => None,
        };

        let Some(expected_category) = expected_category else {
            return false;
        };
        let Some((_, item_ui_category)) = ({
            let mut gamedata = self.gamedata.lock();
            housing_appearance_item_data(&mut gamedata, item)
        }) else {
            return false;
        };

        item_ui_category == expected_category
    }

    fn get_player_or_housing_item(&self, container: ContainerType, slot: u16) -> Option<Item> {
        self.player_data
            .inventory
            .get_item(container, slot)
            .or_else(|| self.player_data.house_inventory.get_item(container, slot))
    }

    fn set_player_or_housing_item(
        &mut self,
        container: ContainerType,
        slot: u16,
        item: Item,
    ) -> bool {
        if let Some(dst) = self.player_data.inventory.get_item_mut(container, slot) {
            *dst = item;
            return true;
        }
        if let Some(dst) = self
            .player_data
            .house_inventory
            .get_item_mut(container, slot)
        {
            *dst = item;
            return true;
        }

        false
    }

    async fn send_current_housing_interior_details(&mut self) {
        let Some(active_estate) = self.active_housing_estate.as_ref() else {
            return;
        };
        let Some((interior_json, light_level, is_apartment)) = ({
            let mut database = self.database.lock();
            database
                .housing_estate_by_house_id(active_estate.house_id)
                .map(|estate| {
                    (
                        estate.interior_json,
                        estate.light_level.clamp(0, u8::MAX as i32) as u8,
                        estate.is_apartment,
                    )
                })
        }) else {
            return;
        };

        self.send_housing_interior_details(&interior_json, light_level, is_apartment)
            .await;
    }

    pub(super) async fn send_housing_interior_details(
        &mut self,
        interior_json: &str,
        light_level: u8,
        is_apartment: bool,
    ) {
        let details = housing_interior_details_from_json(interior_json, light_level, is_apartment);

        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::HousingInteriorDetails(details));
        self.send_ipc_self(ipc).await;
    }

    pub(super) async fn send_housing_furniture_lists(
        &mut self,
        house_id: HouseId,
        rows: &[HousingFurniture],
        indoors: bool,
        slot_capacity: Option<usize>,
    ) {
        let lists = build_furniture_lists(house_id, rows, indoors, slot_capacity);
        self.send_built_housing_furniture_lists(
            lists,
            house_id,
            rows.len(),
            indoors,
            slot_capacity,
            0,
            usize::MAX,
            "all",
        )
        .await;
    }

    pub(super) async fn send_deferred_housing_furniture_lists(
        &mut self,
        house_id: HouseId,
        rows: &[HousingFurniture],
        indoors: bool,
        slot_capacity: Option<usize>,
    ) {
        let lists = build_furniture_lists(house_id, rows, indoors, slot_capacity);
        let start_index = deferred_housing_furniture_list_start_index(indoors, lists.len());
        self.send_built_housing_furniture_lists(
            lists,
            house_id,
            rows.len(),
            indoors,
            slot_capacity,
            start_index,
            usize::MAX,
            "after_remodel_gate",
        )
        .await;
    }

    pub(super) async fn send_built_housing_furniture_lists(
        &mut self,
        lists: Vec<FurnitureList>,
        house_id: HouseId,
        furniture_count: usize,
        indoors: bool,
        slot_capacity: Option<usize>,
        start_index: usize,
        end_index: usize,
        phase: &'static str,
    ) {
        let list_count = lists.len();
        let start_index = start_index.min(list_count);
        let end_index = end_index.min(list_count);
        let send_count = end_index.saturating_sub(start_index);

        tracing::debug!(
            house_id = house_id.to_u64(),
            furniture_count,
            list_count,
            start_index,
            end_index,
            send_count,
            indoors,
            slot_capacity,
            phase,
            "Sending housing furniture lists"
        );

        for list in lists.into_iter().skip(start_index).take(send_count) {
            self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                list,
            )))
            .await;
        }
    }

    pub async fn persist_furniture_placement(
        &mut self,
        container: ContainerType,
        slot: u16,
        position: Position,
        rotation: f32,
        spawn_furniture: bool,
        plot_index: u8,
        intended_use: TerritoryIntendedUse,
    ) -> Option<PersistedFurniturePlacement> {
        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_edit(plot_index)
        } else {
            self.active_housing_estate_for_edit(intended_use)
        };
        let Some(active_estate) = active_estate else {
            return None;
        };

        let transfer_item = match self.housing_transfer_item(container, slot) {
            Some(item) if !item.is_empty_slot() => item,
            _ => {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    ?container,
                    slot,
                    "Rejecting furniture placement from an empty or invalid source slot"
                );
                return None;
            }
        };

        let catalog_id = {
            let gamedata = self.gamedata.lock();
            gamedata
                .get_furniture_catalog_id(transfer_item.item_id)
                .unwrap_or_default()
        };
        if catalog_id == 0 {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                item_id = transfer_item.item_id,
                "Furniture catalog id resolved to zero"
            );
        }

        let desired_pages = self
            .player_data
            .house_inventory
            .get_desired_pages_from_intendeduse(intended_use, !spawn_furniture);
        let Some(result) = self
            .player_data
            .house_inventory
            .add_in_empty_slot(transfer_item, desired_pages)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?desired_pages,
                "Rejecting furniture placement because the target housing inventory is full"
            );
            return None;
        };
        let object_slot = if spawn_furniture {
            let Some(object_slot) = flat_slot_for_container(result.container, result.slot) else {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    dst_container = ?result.container,
                    dst_slot = result.slot,
                    "Rejecting spawned furniture placement because destination is not a placed furniture container"
                );
                return None;
            };
            object_slot
        } else {
            0
        };

        self.clear_housing_transfer_source(container, slot).await?;

        let src_container_type = container;
        let dst_container_type = result.container;
        let stain = transfer_item.stains[0];
        {
            let mut database = self.database.lock();
            database.upsert_housing_furniture(HousingFurniture {
                land_ident: active_estate.land_ident,
                container_type: result.container as u16 as i32,
                slot: result.slot as i32,
                item_id: transfer_item.item_id as i64,
                catalog_id: catalog_id as i32,
                stain: stain as i32,
                placed: spawn_furniture,
                pos_x: position.0.x,
                pos_y: position.0.y,
                pos_z: position.0.z,
                rotation,
                created_by_content_id: Some(self.player_data.character.content_id),
                ..Default::default()
            });
            database.commit_classjob_and_inventory(&self.player_data);
        }

        self.send_affected_containers(src_container_type, dst_container_type)
            .await;

        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            ?container,
            slot,
            dst_container = ?result.container,
            dst_slot = result.slot,
            catalog_id,
            spawn_furniture,
            "Persisted housing furniture placement"
        );

        Some(PersistedFurniturePlacement {
            container: result.container,
            slot: result.slot,
            catalog_id,
            stain,
            position,
            indoors: intended_use == TerritoryIntendedUse::HousingIndoor,
            rotation,
            ward_index: if intended_use == TerritoryIntendedUse::HousingIndoor {
                0
            } else {
                active_estate.house_id.ward_index
            },
            plot_index,
            object_slot,
            spawned: spawn_furniture,
        })
    }

    pub fn persist_furniture_translation(
        &mut self,
        house_id: HouseId,
        flat_slot: u16,
        position: Position,
        rotation: f32,
        intended_use: TerritoryIntendedUse,
    ) -> Option<PersistedFurnitureTranslation> {
        let indoors = intended_use == TerritoryIntendedUse::HousingIndoor;
        let Some((storage_id, container_slot)) = placed_container_for_flat_slot(flat_slot, indoors)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                flat_slot,
                indoors,
                "Rejecting furniture translation for an out-of-range slot"
            );
            return None;
        };

        let active_estate = if indoors {
            self.active_housing_estate_for_edit(intended_use)
        } else {
            self.active_housing_estate_for_outdoor_edit(house_id.unit.apartment_division_plot_index)
        };
        let Some(active_estate) = active_estate else {
            return None;
        };

        if active_estate.house_id != house_id {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                active_house_id = active_estate.house_id.to_u64(),
                packet_house_id = house_id.to_u64(),
                "Rejecting furniture translation for a different house id than the active estate"
            );
            return None;
        }

        let updated = {
            let mut database = self.database.lock();
            database.update_housing_furniture_position(
                active_estate.land_ident,
                storage_id,
                container_slot,
                position,
                rotation,
            )
        };
        if !updated {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                land_ident = active_estate.land_ident,
                ?storage_id,
                container_slot,
                "Furniture translation did not match a persisted furniture row"
            );
            return None;
        }

        Some(PersistedFurnitureTranslation {
            storage_id,
            container_slot,
            plot_number: if !house_id.unit.apartment_flag {
                house_id.unit.apartment_division_plot_index as u16
            } else {
                0
            },
            object_key: HousingFurnitureObjectKey {
                slot: flat_slot,
                indoors,
                ward_index: if indoors { 0 } else { house_id.ward_index },
                plot_index: house_id.unit.apartment_division_plot_index,
            },
        })
    }

    pub async fn persist_housing_item_move_to_inventory(
        &mut self,
        to_storeroom: bool,
        storage_id: ContainerType,
        slot: u16,
        intended_use: TerritoryIntendedUse,
    ) -> Option<PersistedHousingItemMove> {
        let active_estate = if intended_use == TerritoryIntendedUse::HousingOutdoor {
            self.active_housing_estate_for_outdoor_item_removal()
        } else {
            self.active_housing_estate_for_edit(intended_use)
        };
        let Some(active_estate) = active_estate else {
            return None;
        };

        let Some(transfer_item) = self.player_data.house_inventory.get_item(storage_id, slot)
        else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?storage_id,
                slot,
                "Rejecting housing item move from an invalid source slot"
            );
            return None;
        };
        if transfer_item.is_empty_slot() {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?storage_id,
                slot,
                "Rejecting housing item move from an empty source slot"
            );
            return None;
        }
        let item_id = transfer_item.item_id;
        let removed_from_world = should_broadcast_housing_item_removal(storage_id);
        let object_key = if removed_from_world {
            flat_slot_for_container(storage_id, slot).map(|flat_slot| HousingFurnitureObjectKey {
                slot: flat_slot,
                indoors: intended_use == TerritoryIntendedUse::HousingIndoor,
                ward_index: if intended_use == TerritoryIntendedUse::HousingIndoor {
                    0
                } else {
                    active_estate.house_id.ward_index
                },
                plot_index: active_estate.house_id.unit.apartment_division_plot_index,
            })
        } else {
            None
        };

        let desired_pages = self
            .player_data
            .house_inventory
            .get_desired_pages_from_intendeduse(intended_use, to_storeroom);
        let item_info = if !to_storeroom {
            self.player_data
                .inventory
                .add_in_next_free_slot(transfer_item)
        } else {
            self.player_data
                .house_inventory
                .add_in_empty_slot(transfer_item, desired_pages)
        };
        let Some(item_info) = item_info else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?desired_pages,
                to_storeroom,
                "Rejecting housing item move because destination inventory is full"
            );
            return None;
        };

        let source_slot = self
            .player_data
            .house_inventory
            .get_item_mut(storage_id, slot)?;
        *source_slot = Item::default();

        {
            let mut database = self.database.lock();
            let moved = if to_storeroom {
                database.move_housing_furniture_to_container(
                    active_estate.land_ident,
                    storage_id,
                    slot,
                    Some(item_info.container),
                    Some(item_info.slot),
                    false,
                )
            } else {
                database.move_housing_furniture_to_container(
                    active_estate.land_ident,
                    storage_id,
                    slot,
                    None,
                    None,
                    false,
                )
            };

            if !moved {
                tracing::warn!(
                    content_id = self.player_data.character.content_id,
                    land_ident = active_estate.land_ident,
                    ?storage_id,
                    slot,
                    to_storeroom,
                    "Housing item move did not match a persisted furniture row"
                );
            }

            database.commit_classjob_and_inventory(&self.player_data);
        }

        self.send_affected_containers(storage_id, item_info.container)
            .await;

        tracing::debug!(
            content_id = self.player_data.character.content_id,
            land_ident = active_estate.land_ident,
            ?storage_id,
            slot,
            dst_container = ?item_info.container,
            dst_slot = item_info.slot,
            to_storeroom,
            "Persisted housing item move"
        );

        Some(PersistedHousingItemMove {
            item_id,
            src_container: storage_id,
            dst_container: item_info.container,
            ack_slot: slot,
            removed_from_world,
            object_key,
        })
    }

    fn housing_transfer_item(&self, container: ContainerType, slot: u16) -> Option<Item> {
        self.player_data
            .inventory
            .get_item(container, slot)
            .or_else(|| self.player_data.house_inventory.get_item(container, slot))
    }

    async fn clear_housing_transfer_source(
        &mut self,
        container: ContainerType,
        slot: u16,
    ) -> Option<()> {
        let old_slot = if let Some(item) = self.player_data.inventory.get_item_mut(container, slot)
        {
            item
        } else if let Some(item) = self
            .player_data
            .house_inventory
            .get_item_mut(container, slot)
        {
            item
        } else {
            tracing::warn!(
                content_id = self.player_data.character.content_id,
                ?container,
                slot,
                "Unable to clear housing furniture source slot"
            );
            return None;
        };

        *old_slot = Item::default();

        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
            sequence: 0,
            container,
            slot,
            ..Item::default().into()
        }));
        self.send_ipc_self(ipc).await;

        Some(())
    }
}

pub(super) const ITEM_UI_CATEGORY_ROOF: u8 = 65;
pub(super) const ITEM_UI_CATEGORY_EXTERIOR_WALL: u8 = 66;
pub(super) const ITEM_UI_CATEGORY_WINDOW: u8 = 67;
pub(super) const ITEM_UI_CATEGORY_DOOR: u8 = 68;
pub(super) const ITEM_UI_CATEGORY_ROOF_DECORATION: u8 = 69;
pub(super) const ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION: u8 = 70;
pub(super) const ITEM_UI_CATEGORY_PLACARD: u8 = 71;
pub(super) const ITEM_UI_CATEGORY_FENCE: u8 = 72;
pub(super) const ITEM_UI_CATEGORY_INTERIOR_WALL: u8 = 73;
pub(super) const ITEM_UI_CATEGORY_FLOORING: u8 = 74;
pub(super) const ITEM_UI_CATEGORY_CEILING_LIGHT: u8 = 75;

pub(super) fn build_estate_furniture_lists(
    estate: &HousingEstate,
    rows: &[HousingFurniture],
    indoors: bool,
) -> Vec<FurnitureList> {
    let house_id = if indoors {
        HouseId::from_u64(estate.house_id as u64)
    } else {
        outdoor_house_id_from_estate(estate)
    };

    build_furniture_lists(house_id, rows, indoors, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HousingAppearanceSlotSpec {
    pub(super) container: ContainerType,
    pub(super) slot: u16,
    pub(super) additional_data: u32,
    pub(super) item_ui_category: u8,
    pub(super) stain: u8,
}

pub(super) fn populate_housing_appearance_inventory(
    inventory: &mut HousingInventory,
    game_data: &mut GameData,
    estate: &HousingEstate,
) {
    let exterior = house_exterior_from_json(&estate.exterior_json);
    let light_level = estate.light_level.clamp(0, u8::MAX as i32) as u8;
    let interior =
        housing_interior_details_from_json(&estate.interior_json, light_level, estate.is_apartment);

    for spec in housing_exterior_appearance_slot_specs(&exterior)
        .into_iter()
        .chain(housing_interior_appearance_slot_specs(
            &interior,
            estate.is_apartment,
        ))
    {
        populate_housing_appearance_slot(inventory, game_data, spec);
    }
}

fn populate_housing_appearance_slot(
    inventory: &mut HousingInventory,
    game_data: &mut GameData,
    spec: HousingAppearanceSlotSpec,
) {
    let Some(item_id) =
        game_data.get_item_id_by_additional_data(spec.additional_data, spec.item_ui_category)
    else {
        tracing::warn!(
            ?spec.container,
            slot = spec.slot,
            additional_data = spec.additional_data,
            item_ui_category = spec.item_ui_category,
            "Unable to resolve housing appearance fixture item"
        );
        return;
    };

    let Some(slot) = inventory.get_item_mut(spec.container, spec.slot) else {
        tracing::warn!(
            ?spec.container,
            slot = spec.slot,
            item_id,
            "Unable to populate housing appearance slot"
        );
        return;
    };

    *slot = Item {
        quantity: 1,
        item_id,
        condition: ITEM_CONDITION_MAX,
        stains: [spec.stain, 0],
        ..Default::default()
    };
}

pub(super) fn housing_exterior_appearance_slot_specs(
    exterior: &HouseExterior,
) -> Vec<HousingAppearanceSlotSpec> {
    let container = ContainerType::HousingExteriorAppearance;
    vec![
        HousingAppearanceSlotSpec {
            container,
            slot: 1,
            additional_data: exterior.roof_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_ROOF,
            stain: exterior.colors.roof,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 2,
            additional_data: exterior.walls_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_EXTERIOR_WALL,
            stain: exterior.colors.walls,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 3,
            additional_data: exterior.windows_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_WINDOW,
            stain: exterior.colors.windows,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 4,
            additional_data: exterior.door_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_DOOR,
            stain: exterior.colors.door,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 5,
            additional_data: exterior.roof_fixture_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_ROOF_DECORATION,
            stain: exterior.colors.roof_fixture,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 6,
            additional_data: exterior.wall_fixture_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION,
            stain: exterior.colors.wall_fixture,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 7,
            additional_data: exterior.above_door_banner_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_PLACARD,
            stain: exterior.colors.above_door_banner,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 8,
            additional_data: exterior.fence_id as u32,
            item_ui_category: ITEM_UI_CATEGORY_FENCE,
            stain: exterior.colors.fence,
        },
    ]
    .into_iter()
    .filter(|spec| spec.additional_data != 0)
    .collect()
}

pub(super) fn housing_interior_appearance_slot_specs(
    interior: &HousingInteriorDetails,
    is_apartment: bool,
) -> Vec<HousingAppearanceSlotSpec> {
    let container = ContainerType::HousingInteriorAppearance;
    let mut specs = vec![
        HousingAppearanceSlotSpec {
            container,
            slot: 0,
            additional_data: interior.ground_walls,
            item_ui_category: ITEM_UI_CATEGORY_INTERIOR_WALL,
            stain: 0,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 1,
            additional_data: interior.ground_floor,
            item_ui_category: ITEM_UI_CATEGORY_FLOORING,
            stain: 0,
        },
        HousingAppearanceSlotSpec {
            container,
            slot: 2,
            additional_data: interior.ground_chandelier,
            item_ui_category: ITEM_UI_CATEGORY_CEILING_LIGHT,
            stain: 0,
        },
    ];

    if !is_apartment {
        specs.extend([
            HousingAppearanceSlotSpec {
                container,
                slot: 3,
                additional_data: interior.top_walls,
                item_ui_category: ITEM_UI_CATEGORY_INTERIOR_WALL,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 4,
                additional_data: interior.top_floor,
                item_ui_category: ITEM_UI_CATEGORY_FLOORING,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 5,
                additional_data: interior.top_chandelier,
                item_ui_category: ITEM_UI_CATEGORY_CEILING_LIGHT,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 6,
                additional_data: interior.cellar_walls,
                item_ui_category: ITEM_UI_CATEGORY_INTERIOR_WALL,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 7,
                additional_data: interior.cellar_floor,
                item_ui_category: ITEM_UI_CATEGORY_FLOORING,
                stain: 0,
            },
            HousingAppearanceSlotSpec {
                container,
                slot: 8,
                additional_data: interior.cellar_chandelier,
                item_ui_category: ITEM_UI_CATEGORY_CEILING_LIGHT,
                stain: 0,
            },
        ]);
    }

    specs
        .into_iter()
        .filter(|spec| spec.additional_data != 0)
        .collect()
}

pub(super) fn placed_container_for_flat_slot(
    flat_slot: u16,
    indoors: bool,
) -> Option<(ContainerType, u16)> {
    if indoors {
        return indoor_container_for_flat_slot(flat_slot);
    }

    if flat_slot < 50 {
        Some((ContainerType::HousingExteriorPlacedItems, flat_slot))
    } else {
        None
    }
}

pub(super) fn should_broadcast_housing_item_removal(container: ContainerType) -> bool {
    container == ContainerType::HousingExteriorPlacedItems
        || is_interior_placed_container(container)
}

fn is_housing_appearance_container(container: ContainerType) -> bool {
    matches!(
        container,
        ContainerType::HousingExteriorAppearance
            | ContainerType::HousingExteriorAppearanceEdit
            | ContainerType::HousingInteriorAppearance
            | ContainerType::HousingInteriorAppearanceEdit
    )
}

fn housing_appearance_container_intended_use(
    container: ContainerType,
) -> Option<TerritoryIntendedUse> {
    match container {
        ContainerType::HousingExteriorAppearance | ContainerType::HousingExteriorAppearanceEdit => {
            Some(TerritoryIntendedUse::HousingOutdoor)
        }
        ContainerType::HousingInteriorAppearance | ContainerType::HousingInteriorAppearanceEdit => {
            Some(TerritoryIntendedUse::HousingIndoor)
        }
        _ => None,
    }
}

fn housing_appearance_operation_involves_staging(action: &ItemOperation) -> bool {
    is_housing_appearance_container(action.src_storage_id)
        || is_housing_appearance_container(action.dst_storage_id)
}

fn housing_appearance_operation_matches_area(
    action: &ItemOperation,
    intended_use: TerritoryIntendedUse,
) -> bool {
    [action.src_storage_id, action.dst_storage_id]
        .into_iter()
        .filter_map(housing_appearance_container_intended_use)
        .all(|container_use| container_use == intended_use)
}

pub(super) fn default_housing_appearance_target_slot(
    item_ui_category: u8,
    intended_use: TerritoryIntendedUse,
    is_apartment: bool,
) -> Option<u16> {
    match intended_use {
        TerritoryIntendedUse::HousingIndoor => {
            let base_slot = if is_apartment { 0 } else { 3 };
            match item_ui_category {
                ITEM_UI_CATEGORY_INTERIOR_WALL => Some(base_slot),
                ITEM_UI_CATEGORY_FLOORING => Some(base_slot + 1),
                ITEM_UI_CATEGORY_CEILING_LIGHT => Some(base_slot + 2),
                _ => None,
            }
        }
        TerritoryIntendedUse::HousingOutdoor => match item_ui_category {
            ITEM_UI_CATEGORY_ROOF => Some(1),
            ITEM_UI_CATEGORY_EXTERIOR_WALL => Some(2),
            ITEM_UI_CATEGORY_WINDOW => Some(3),
            ITEM_UI_CATEGORY_DOOR => Some(4),
            ITEM_UI_CATEGORY_ROOF_DECORATION => Some(5),
            ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION => Some(6),
            ITEM_UI_CATEGORY_PLACARD => Some(7),
            ITEM_UI_CATEGORY_FENCE => Some(8),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn housing_appearance_marker_target_slot(
    item_ui_category: u8,
    intended_use: TerritoryIntendedUse,
    is_apartment: bool,
    marker_target_slot: Option<u16>,
) -> Option<u16> {
    let target_slot = marker_target_slot?;
    if intended_use == TerritoryIntendedUse::HousingIndoor && is_apartment && target_slot > 2 {
        return None;
    }

    let expected_category = match intended_use {
        TerritoryIntendedUse::HousingIndoor => {
            housing_interior_field_for_appearance_slot(target_slot)
                .and_then(|_| housing_interior_item_ui_category_for_slot(target_slot))
        }
        TerritoryIntendedUse::HousingOutdoor => {
            housing_exterior_field_for_appearance_slot(target_slot)
                .and_then(|_| housing_exterior_item_ui_category_for_slot(target_slot))
        }
        _ => None,
    };

    (expected_category == Some(item_ui_category)).then_some(target_slot)
}

pub(super) fn housing_appearance_target_slot(
    item_ui_category: u8,
    intended_use: TerritoryIntendedUse,
    is_apartment: bool,
    marker_target_slot: Option<u16>,
) -> Option<u16> {
    match marker_target_slot {
        Some(_) => housing_appearance_marker_target_slot(
            item_ui_category,
            intended_use,
            is_apartment,
            marker_target_slot,
        ),
        None => {
            default_housing_appearance_target_slot(item_ui_category, intended_use, is_apartment)
        }
    }
}

fn housing_appearance_item_data(game_data: &mut GameData, item: Item) -> Option<(u32, u8)> {
    if item.is_empty_slot() {
        return None;
    }

    let row = game_data.get_item_info(ItemInfoQuery::ById(item.item_id))?;
    (row.additional_data != 0).then_some((row.additional_data, row.item_ui_category))
}

pub(super) fn housing_interior_field_for_appearance_slot(
    slot: u16,
) -> Option<HousingInteriorField> {
    match slot {
        0 => Some(HousingInteriorField::GroundWalls),
        1 => Some(HousingInteriorField::GroundFloor),
        2 => Some(HousingInteriorField::GroundChandelier),
        3 => Some(HousingInteriorField::TopWalls),
        4 => Some(HousingInteriorField::TopFloor),
        5 => Some(HousingInteriorField::TopChandelier),
        6 => Some(HousingInteriorField::CellarWalls),
        7 => Some(HousingInteriorField::CellarFloor),
        8 => Some(HousingInteriorField::CellarChandelier),
        _ => None,
    }
}

pub(super) fn housing_interior_item_ui_category_for_slot(slot: u16) -> Option<u8> {
    match slot % 3 {
        0 => Some(ITEM_UI_CATEGORY_INTERIOR_WALL),
        1 => Some(ITEM_UI_CATEGORY_FLOORING),
        2 => Some(ITEM_UI_CATEGORY_CEILING_LIGHT),
        _ => None,
    }
}

pub(super) fn housing_exterior_field_for_appearance_slot(
    slot: u16,
) -> Option<HousingExteriorField> {
    match slot {
        1 => Some(HousingExteriorField::Roof),
        2 => Some(HousingExteriorField::Walls),
        3 => Some(HousingExteriorField::Windows),
        4 => Some(HousingExteriorField::Door),
        5 => Some(HousingExteriorField::RoofFixture),
        6 => Some(HousingExteriorField::WallFixture),
        7 => Some(HousingExteriorField::AboveDoorBanner),
        8 => Some(HousingExteriorField::Fence),
        _ => None,
    }
}

pub(super) fn housing_exterior_item_ui_category_for_slot(slot: u16) -> Option<u8> {
    match slot {
        1 => Some(ITEM_UI_CATEGORY_ROOF),
        2 => Some(ITEM_UI_CATEGORY_EXTERIOR_WALL),
        3 => Some(ITEM_UI_CATEGORY_WINDOW),
        4 => Some(ITEM_UI_CATEGORY_DOOR),
        5 => Some(ITEM_UI_CATEGORY_ROOF_DECORATION),
        6 => Some(ITEM_UI_CATEGORY_EXTERIOR_WALL_DECORATION),
        7 => Some(ITEM_UI_CATEGORY_PLACARD),
        8 => Some(ITEM_UI_CATEGORY_FENCE),
        _ => None,
    }
}

pub(super) fn housing_exterior_color_field_for_appearance_slot(
    slot: u16,
) -> Option<HousingExteriorColorField> {
    match slot {
        1 => Some(HousingExteriorColorField::Roof),
        2 => Some(HousingExteriorColorField::Walls),
        3 => Some(HousingExteriorColorField::Windows),
        4 => Some(HousingExteriorColorField::Door),
        5 => Some(HousingExteriorColorField::RoofFixture),
        6 => Some(HousingExteriorColorField::WallFixture),
        7 => Some(HousingExteriorColorField::AboveDoorBanner),
        8 => Some(HousingExteriorColorField::Fence),
        _ => None,
    }
}
