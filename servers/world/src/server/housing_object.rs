use std::collections::VecDeque;

use crate::StatusEffects;
use crate::common::{HousingFurnitureObject, HousingFurnitureObjectKey};
use crate::gamedata::HousingStrikingDummyNpcData;
use crate::server::actor::{NetworkedActor, NpcState};
use crate::server::instance::Instance;
use crate::server::network::NetworkState;
use kawari::{
    common::{ObjectId, Position, STRIKING_DUMMY_NAME_ID, Timeline},
    ipc::zone::{
        BattleNpcSubKind, CharacterDataFlag, CommonSpawn, ObjectKind,
        SPAWN_OBJECT_TARGETABLE_STATUS_NONE, SpawnNpc, SpawnObject,
    },
};

const HOUSING_ENTITY_PREFIX: u32 = 0x4000_0000;
const HOUSING_STRIKING_DUMMY_ENTITY_PREFIX: u32 = 0x6B00_0000;
const HOUSING_STRIKING_DUMMY_GIMMICK_PREFIX: u32 = 0x6C00_0000;
const HOUSING_OBJECT_TYPE_YARD_OBJECT: u32 = 2;
const HOUSING_OBJECT_TYPE_FURNITURE: u32 = 3;
const HOUSING_STRIKING_DUMMY_LEVEL: u8 = 1;
const HOUSING_STRIKING_DUMMY_LINK_RANGE: u8 = 20;
pub const HOUSING_STRIKING_DUMMY_HEALTH: u32 = 1_000_000_000;

pub fn housing_furniture_tracking_id(key: HousingFurnitureObjectKey) -> u16 {
    if key.indoors {
        key.slot
    } else {
        ((key.plot_index as u16) << 8) | (key.slot & 0x00FF)
    }
}

pub fn housing_furniture_actor_id(key: HousingFurnitureObjectKey) -> ObjectId {
    let tracking_id = housing_furniture_tracking_id(key) as u32;

    ObjectId(HOUSING_ENTITY_PREFIX | tracking_id)
}

pub fn housing_striking_dummy_actor_id(key: HousingFurnitureObjectKey) -> ObjectId {
    let tracking_id = housing_furniture_tracking_id(key) as u32;
    ObjectId(HOUSING_STRIKING_DUMMY_ENTITY_PREFIX | tracking_id)
}

fn housing_striking_dummy_gimmick_id(key: HousingFurnitureObjectKey) -> u32 {
    let tracking_id = housing_furniture_tracking_id(key) as u32;
    HOUSING_STRIKING_DUMMY_GIMMICK_PREFIX | tracking_id
}

pub fn build_housing_furniture_spawn(
    object: HousingFurnitureObject,
    interactable: bool,
) -> Option<(ObjectId, SpawnObject)> {
    if !interactable {
        return None;
    }

    let key = HousingFurnitureObjectKey::from(&object);
    let actor_id = housing_furniture_actor_id(key);
    let object_type = if object.indoors {
        HOUSING_OBJECT_TYPE_FURNITURE
    } else {
        HOUSING_OBJECT_TYPE_YARD_OBJECT
    };
    let base_id = (object_type << 16) | object.catalog_id as u32;

    Some((
        actor_id,
        SpawnObject {
            kind: ObjectKind::HousingEventObject,
            targetable_status: SPAWN_OBJECT_TARGETABLE_STATUS_NONE,
            base_id,
            entity_id: actor_id,
            args2: housing_furniture_tracking_id(key) as u32,
            radius: 1.0,
            rotation: object.rotation,
            position: object.position,
            ..Default::default()
        },
    ))
}

pub fn build_housing_striking_dummy_spawn(
    object: HousingFurnitureObject,
    data: &HousingStrikingDummyNpcData,
) -> Option<(ObjectId, SpawnNpc)> {
    if object.indoors {
        return None;
    }

    let key = HousingFurnitureObjectKey::from(&object);
    let actor_id = housing_striking_dummy_actor_id(key);
    let layout_id = housing_striking_dummy_gimmick_id(key);

    Some((
        actor_id,
        SpawnNpc {
            gimmick_id: layout_id,
            character_data_flags: CharacterDataFlag::empty(),
            character_data_icon: data.rank,
            normal_scaling: false,
            max_links: 0,
            link_family: 0,
            link_range: HOUSING_STRIKING_DUMMY_LINK_RANGE,
            common: CommonSpawn {
                base_id: data.base_id,
                name_id: STRIKING_DUMMY_NAME_ID,
                max_health_points: HOUSING_STRIKING_DUMMY_HEALTH,
                health_points: HOUSING_STRIKING_DUMMY_HEALTH,
                model_chara: data.model_chara,
                object_kind: ObjectKind::BattleNpc(BattleNpcSubKind::Enemy),
                battalion: data.battalion,
                level: HOUSING_STRIKING_DUMMY_LEVEL,
                position: object.position,
                rotation: object.rotation,
                look: data.customize.clone(),
                layout_id,
                ..data.equip.clone()
            },
            ..Default::default()
        },
    ))
}

fn insert_housing_striking_dummy(instance: &mut Instance, actor_id: ObjectId, spawn: SpawnNpc) {
    instance.actors.insert(
        actor_id,
        NetworkedActor::Npc {
            state: NpcState::Stationary,
            navmesh_path: VecDeque::default(),
            navmesh_path_lerp: 0.0,
            navmesh_target: None,
            last_position: None,
            spawn,
            timeline: Timeline {
                autoattack_action_id: 0,
                timeline_always_plays: false,
                timepoints: Vec::new(),
                on_death: Vec::new(),
            },
            timeline_position: 0,
            newly_hated_actor: None,
            currently_invulnerable: false,
            status_effects: StatusEffects::default(),
        },
    );
}

pub fn upsert_housing_furniture_object(
    instance: &mut Instance,
    object: HousingFurnitureObject,
    interactable: bool,
    striking_dummy_data: Option<&HousingStrikingDummyNpcData>,
) -> Option<ObjectId> {
    let key = HousingFurnitureObjectKey::from(&object);

    if !interactable {
        tracing::debug!(
            slot = object.slot,
            catalog_id = object.catalog_id,
            indoors = object.indoors,
            plot_index = object.plot_index,
            "Skipping housing furniture object overlay because row is not interactable"
        );
        remove_housing_furniture_object(instance, key);
        return None;
    }

    if let Some(striking_dummy_data) = striking_dummy_data
        && let Some((actor_id, spawn)) =
            build_housing_striking_dummy_spawn(object, striking_dummy_data)
    {
        instance.actors.remove(&housing_furniture_actor_id(key));
        tracing::debug!(
            actor_id = actor_id.0,
            slot = object.slot,
            catalog_id = object.catalog_id,
            indoors = object.indoors,
            plot_index = object.plot_index,
            position_x = object.position.0.x,
            position_y = object.position.0.y,
            position_z = object.position.0.z,
            "Upserting housing striking dummy battle npc"
        );
        insert_housing_striking_dummy(instance, actor_id, spawn);
        return Some(actor_id);
    }

    instance
        .actors
        .remove(&housing_striking_dummy_actor_id(key));
    let (actor_id, spawn) = build_housing_furniture_spawn(object, interactable)?;
    tracing::debug!(
        actor_id = actor_id.0,
        slot = object.slot,
        catalog_id = object.catalog_id,
        base_id = spawn.base_id,
        indoors = object.indoors,
        plot_index = object.plot_index,
        position_x = object.position.0.x,
        position_y = object.position.0.y,
        position_z = object.position.0.z,
        "Upserting housing furniture object overlay"
    );
    instance.insert_object(actor_id, spawn, String::default());
    Some(actor_id)
}

pub fn remove_housing_furniture_object(
    instance: &mut Instance,
    key: HousingFurnitureObjectKey,
) -> bool {
    let removed_object = instance
        .actors
        .remove(&housing_furniture_actor_id(key))
        .is_some();
    let removed_dummy = instance
        .actors
        .remove(&housing_striking_dummy_actor_id(key))
        .is_some();
    removed_object || removed_dummy
}

pub fn remove_housing_furniture_object_networked(
    instance: &mut Instance,
    network: &mut NetworkState,
    key: HousingFurnitureObjectKey,
) -> bool {
    let mut removed = false;
    for actor_id in [
        housing_furniture_actor_id(key),
        housing_striking_dummy_actor_id(key),
    ] {
        if instance.find_actor(actor_id).is_none() {
            continue;
        }

        network.remove_actor(instance, actor_id);
        removed = true;
    }

    removed
}

pub fn spawn_housing_furniture_object_for_current_clients(
    instance: &Instance,
    network: &mut NetworkState,
    actor_id: ObjectId,
) -> usize {
    let Some(actor) = instance.find_actor(actor_id) else {
        tracing::debug!(
            actor_id = actor_id.0,
            "Skipping housing furniture object spawn because actor is not in instance"
        );
        return 0;
    };
    let mut failed_clients = Vec::new();
    let mut sent_count = 0;

    for (client_id, (handle, state)) in &mut network.clients {
        let Some(viewer) = instance.find_actor(handle.actor_id) else {
            tracing::debug!(
                client_id = ?client_id,
                actor_id = actor_id.0,
                viewer_actor_id = handle.actor_id.0,
                "Skipping housing furniture object spawn because viewer actor is not in instance"
            );
            continue;
        };
        if !viewer.is_valid() {
            tracing::debug!(
                client_id = ?client_id,
                actor_id = actor_id.0,
                viewer_actor_id = handle.actor_id.0,
                "Skipping housing furniture object spawn because viewer is not valid yet"
            );
            continue;
        }
        if !viewer.in_range_of(actor) {
            tracing::debug!(
                client_id = ?client_id,
                actor_id = actor_id.0,
                viewer_actor_id = handle.actor_id.0,
                "Skipping housing furniture object spawn because it is out of viewer range"
            );
            continue;
        }
        if state.has_spawned(actor_id) {
            tracing::debug!(
                client_id = ?client_id,
                actor_id = actor_id.0,
                "Skipping housing furniture object spawn because client already has it"
            );
            continue;
        }

        if let Some(message) = NetworkState::spawn_existing_actor_message(state, actor_id, actor) {
            tracing::debug!(
                client_id = ?client_id,
                actor_id = actor_id.0,
            "Sending housing furniture object spawn to client"
            );
            if handle.send(message).is_err() {
                failed_clients.push(*client_id);
            } else {
                sent_count += 1;
            }
        } else {
            tracing::debug!(
                client_id = ?client_id,
                actor_id = actor_id.0,
                "Skipping housing furniture object spawn because client allocator is full"
            );
        }
    }

    network.to_remove.extend(failed_clients);
    sent_count
}

pub fn update_housing_furniture_object_position_networked(
    instance: &mut Instance,
    network: &mut NetworkState,
    key: HousingFurnitureObjectKey,
    position: Position,
    rotation: f32,
) -> bool {
    let actor_id = housing_furniture_actor_id(key);
    if let Some(NetworkedActor::Object { object, .. }) = instance.find_actor(actor_id) {
        let mut object = *object;
        object.position = position;
        object.rotation = rotation;

        network.remove_actor(instance, actor_id);
        instance.insert_object(actor_id, object, String::default());
        spawn_housing_furniture_object_for_current_clients(instance, network, actor_id);
        return true;
    }

    let actor_id = housing_striking_dummy_actor_id(key);
    let Some(NetworkedActor::Npc { spawn, .. }) = instance.find_actor(actor_id) else {
        return false;
    };

    let mut spawn = spawn.clone();
    spawn.common.position = position;
    spawn.common.rotation = rotation;

    network.remove_actor(instance, actor_id);
    insert_housing_striking_dummy(instance, actor_id, spawn);
    spawn_housing_furniture_object_for_current_clients(instance, network, actor_id);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamedata::{HOUSING_STRIKING_DUMMY_BNPC_BASE_ID, HousingStrikingDummyNpcData};
    use binrw::BinWrite;
    use glam::Vec3;
    use kawari::{
        common::{CustomizeData, Position, STRIKING_DUMMY_NAME_ID},
        ipc::zone::{BattleNpcSubKind, CommonSpawn, ObjectKind},
    };
    use std::io::Cursor;

    fn furniture(slot: u16, indoors: bool, plot_index: u8) -> HousingFurnitureObject {
        HousingFurnitureObject {
            slot,
            catalog_id: 123,
            position: Position(Vec3::new(1.0, 2.0, 3.0)),
            rotation: 1.25,
            indoors,
            plot_index,
        }
    }

    fn dummy_data() -> HousingStrikingDummyNpcData {
        HousingStrikingDummyNpcData {
            base_id: HOUSING_STRIKING_DUMMY_BNPC_BASE_ID,
            model_chara: 777,
            battalion: 4,
            customize: CustomizeData::default(),
            rank: 0,
            equip: CommonSpawn::default(),
        }
    }

    #[test]
    fn indoor_tracking_id_uses_flat_furniture_slot() {
        let key = HousingFurnitureObjectKey {
            slot: 51,
            indoors: true,
            plot_index: 7,
        };

        assert_eq!(housing_furniture_tracking_id(key), 51);
    }

    #[test]
    fn indoor_actor_id_uses_capture_observed_dynamic_object_range() {
        let key = HousingFurnitureObjectKey {
            slot: 51,
            indoors: true,
            plot_index: 7,
        };

        let actor_id = housing_furniture_actor_id(key);

        assert_eq!(actor_id.0, 0x4000_0033);
    }

    #[test]
    fn outdoor_tracking_id_packs_plot_and_slot() {
        let key = HousingFurnitureObjectKey {
            slot: 9,
            indoors: false,
            plot_index: 5,
        };

        assert_eq!(housing_furniture_tracking_id(key), 0x0509);
    }

    #[test]
    fn indoor_spawn_object_uses_capture_observed_catalog_id_and_slot_args() {
        let object = furniture(51, true, 0);

        let (actor_id, spawn) = build_housing_furniture_spawn(object, true).unwrap();

        assert_eq!(actor_id, housing_furniture_actor_id((&object).into()));
        assert_eq!(spawn.entity_id, actor_id);
        assert_eq!(spawn.kind, ObjectKind::HousingEventObject);
        assert_eq!(spawn.targetable_status, 0x00);
        assert_eq!(spawn.base_id, 0x0003_007B);
        assert_eq!(spawn.args2, 51);
        assert_eq!(spawn.position, object.position);
        assert_eq!(spawn.rotation, object.rotation);
    }

    #[test]
    fn indoor_spawn_object_writes_capture_observed_targetable_status() {
        let object = furniture(51, true, 0);
        let (_, spawn) = build_housing_furniture_spawn(object, true).unwrap();
        let mut buffer = Cursor::new(Vec::new());

        spawn.write_le(&mut buffer).unwrap();

        let bytes = buffer.into_inner();
        assert_eq!(bytes[2], 0x00);
    }

    #[test]
    fn outdoor_spawn_object_uses_yard_object_type_and_catalog_id() {
        let object = furniture(9, false, 5);

        let (_, spawn) = build_housing_furniture_spawn(object, true).unwrap();

        assert_eq!(spawn.base_id, 0x0002_007B);
    }

    #[test]
    fn non_interactable_furniture_does_not_spawn_overlay_object() {
        assert!(build_housing_furniture_spawn(furniture(10, true, 0), false).is_none());
    }

    #[test]
    fn upsert_inserts_targetable_housing_object_into_instance() {
        let mut instance = Instance::default();
        let object = furniture(10, true, 0);

        let actor_id = upsert_housing_furniture_object(&mut instance, object, true, None).unwrap();

        let Some(NetworkedActor::Object { object: spawn, .. }) = instance.find_actor(actor_id)
        else {
            panic!("housing furniture actor should be inserted as an object");
        };
        assert_eq!(spawn.position, object.position);
    }

    #[test]
    fn upsert_striking_dummy_inserts_battle_npc_instead_of_housing_event_object() {
        let mut instance = Instance::default();
        let object = furniture(9, false, 5);

        let actor_id =
            upsert_housing_furniture_object(&mut instance, object, true, Some(&dummy_data()))
                .unwrap();

        assert_eq!(
            actor_id,
            housing_striking_dummy_actor_id(HousingFurnitureObjectKey::from(&object))
        );
        assert!(
            instance
                .find_actor(housing_furniture_actor_id((&object).into()))
                .is_none()
        );

        let Some(NetworkedActor::Npc { state, spawn, .. }) = instance.find_actor(actor_id) else {
            panic!("striking dummy furniture should be inserted as a battle npc");
        };
        assert_eq!(*state, NpcState::Stationary);
        assert_eq!(spawn.common.base_id, HOUSING_STRIKING_DUMMY_BNPC_BASE_ID);
        assert_eq!(spawn.common.name_id, STRIKING_DUMMY_NAME_ID);
        assert_eq!(
            spawn.common.object_kind,
            ObjectKind::BattleNpc(BattleNpcSubKind::Enemy)
        );
        assert_eq!(spawn.common.position, object.position);
        assert_eq!(spawn.common.rotation, object.rotation);
        assert_eq!(spawn.common.model_chara, 777);
        assert_eq!(spawn.common.battalion, 4);
        assert_eq!(spawn.common.health_points, HOUSING_STRIKING_DUMMY_HEALTH);
        assert!(!spawn.normal_scaling);
    }

    #[test]
    fn upsert_replaces_existing_housing_object_position() {
        let mut instance = Instance::default();
        let first = furniture(10, true, 0);
        let mut second = first;
        second.position = Position(Vec3::new(9.0, 8.0, 7.0));

        let actor_id = upsert_housing_furniture_object(&mut instance, first, true, None).unwrap();
        let replaced_actor_id =
            upsert_housing_furniture_object(&mut instance, second, true, None).unwrap();

        assert_eq!(actor_id, replaced_actor_id);
        let Some(NetworkedActor::Object { object: spawn, .. }) = instance.find_actor(actor_id)
        else {
            panic!("housing furniture actor should still be present");
        };
        assert_eq!(spawn.position, second.position);
    }

    #[test]
    fn remove_deletes_matching_housing_object_from_instance() {
        let mut instance = Instance::default();
        let object = furniture(10, true, 0);
        let key = HousingFurnitureObjectKey::from(&object);
        let actor_id = upsert_housing_furniture_object(&mut instance, object, true, None).unwrap();

        assert!(remove_housing_furniture_object(&mut instance, key));

        assert!(instance.find_actor(actor_id).is_none());
    }

    #[test]
    fn non_interactable_upsert_removes_existing_housing_object() {
        let mut instance = Instance::default();
        let object = furniture(10, true, 0);
        let actor_id = upsert_housing_furniture_object(&mut instance, object, true, None).unwrap();

        assert!(upsert_housing_furniture_object(&mut instance, object, false, None).is_none());

        assert!(instance.find_actor(actor_id).is_none());
    }

    #[test]
    fn non_interactable_upsert_removes_existing_striking_dummy() {
        let mut instance = Instance::default();
        let object = furniture(9, false, 5);
        let actor_id =
            upsert_housing_furniture_object(&mut instance, object, true, Some(&dummy_data()))
                .unwrap();

        assert!(upsert_housing_furniture_object(&mut instance, object, false, None).is_none());

        assert!(instance.find_actor(actor_id).is_none());
    }

    #[test]
    fn networked_position_update_replaces_object_with_updated_transform() {
        let mut instance = Instance::default();
        let mut network = NetworkState::default();
        let object = furniture(10, true, 0);
        let key = HousingFurnitureObjectKey::from(&object);
        let actor_id = upsert_housing_furniture_object(&mut instance, object, true, None).unwrap();
        let new_position = Position(Vec3::new(4.0, 5.0, 6.0));
        let new_rotation = 2.5;

        assert!(update_housing_furniture_object_position_networked(
            &mut instance,
            &mut network,
            key,
            new_position,
            new_rotation,
        ));

        let Some(NetworkedActor::Object { object: spawn, .. }) = instance.find_actor(actor_id)
        else {
            panic!("housing furniture actor should still be present");
        };
        assert_eq!(spawn.position, new_position);
        assert_eq!(spawn.rotation, new_rotation);
    }

    #[test]
    fn networked_position_update_replaces_striking_dummy_with_updated_transform() {
        let mut instance = Instance::default();
        let mut network = NetworkState::default();
        let object = furniture(9, false, 5);
        let key = HousingFurnitureObjectKey::from(&object);
        let actor_id =
            upsert_housing_furniture_object(&mut instance, object, true, Some(&dummy_data()))
                .unwrap();
        let new_position = Position(Vec3::new(4.0, 5.0, 6.0));
        let new_rotation = 2.5;

        assert!(update_housing_furniture_object_position_networked(
            &mut instance,
            &mut network,
            key,
            new_position,
            new_rotation,
        ));

        let Some(NetworkedActor::Npc { spawn, .. }) = instance.find_actor(actor_id) else {
            panic!("striking dummy actor should still be present");
        };
        assert_eq!(spawn.common.position, new_position);
        assert_eq!(spawn.common.rotation, new_rotation);
    }
}
