use std::collections::HashMap;

use mlua::{UserData, UserDataFields};

use kawari::{
    common::{ObjectId, ObjectTypeId},
    ipc::zone::ServerZoneIpcSegment,
    packet::PacketSegment,
};

use super::QueueSegments;

#[derive(Default, Debug, Clone)]
pub struct LuaZone {
    pub zone_id: u16,
    pub weather_id: u16,
    pub internal_name: String,
    pub region_name: String,
    pub place_name: String,
    pub intended_use: u8,
    pub map_id: u16,
    pub queued_segments: Vec<PacketSegment<ServerZoneIpcSegment>>,
    // NOTE: These are here to be accessed in Lua via the injected BASE_ID
    pub cached_npc_base_ids: HashMap<ObjectId, u32>,
    pub cached_eobj_base_ids: HashMap<ObjectId, u32>,
    pub cached_eobj_args2: HashMap<ObjectId, u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HousingPlacardLocation {
    pub division: u8,
    pub plot_index: u8,
}

pub fn talk_event_arg_for_actor(zone: &LuaZone, actor_id: ObjectTypeId) -> u32 {
    zone.cached_eobj_args2
        .get(&actor_id.object_id)
        .copied()
        .unwrap_or_default()
}

pub fn housing_placard_location_from_event_arg(
    event_arg: u32,
    fallback_division: u8,
) -> HousingPlacardLocation {
    let raw_index = housing_raw_entry_index_from_event_arg(event_arg);

    if event_arg < 256 {
        return HousingPlacardLocation {
            division: fallback_division,
            plot_index: raw_index,
        };
    }

    match raw_index {
        0..=29 => HousingPlacardLocation {
            division: 0,
            plot_index: raw_index,
        },
        30..=59 => HousingPlacardLocation {
            division: 1,
            plot_index: raw_index - 30,
        },
        _ => HousingPlacardLocation {
            division: fallback_division,
            plot_index: raw_index % 30,
        },
    }
}

fn housing_raw_entry_index_from_event_arg(event_arg: u32) -> u8 {
    if event_arg < 256 {
        event_arg as u8
    } else {
        event_arg.to_le_bytes()[1]
    }
}

impl UserData for LuaZone {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.zone_id));
        fields.add_field_method_get("weather_id", |_, this| Ok(this.weather_id));
        fields.add_field_method_get("internal_name", |_, this| Ok(this.internal_name.clone()));
        fields.add_field_method_get("region_name", |_, this| Ok(this.region_name.clone()));
        fields.add_field_method_get("place_name", |_, this| Ok(this.place_name.clone()));
        fields.add_field_method_get("intended_use", |_, this| Ok(this.intended_use));
    }
}

impl QueueSegments for LuaZone {
    fn queue_segment(&mut self, segment: PacketSegment<ServerZoneIpcSegment>) {
        self.queued_segments.push(segment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawari::common::{ObjectTypeId, ObjectTypeKind};

    #[test]
    fn housing_raw_entry_index_from_packed_event_arg_uses_second_byte() {
        let event_arg = u32::from_le_bytes([0, 5, 0, 0]);

        assert_eq!(event_arg % 256, 0);
        assert_eq!(housing_raw_entry_index_from_event_arg(event_arg), 5);
    }

    #[test]
    fn housing_talk_event_arg_for_actor_uses_cached_eobj_args2() {
        let actor_id = ObjectId(0x1234);
        let mut zone = LuaZone::default();
        let packed_plot_five = u32::from_le_bytes([0, 5, 0, 0]);
        zone.cached_eobj_args2.insert(actor_id, packed_plot_five);

        let event_arg = talk_event_arg_for_actor(
            &zone,
            ObjectTypeId {
                object_id: actor_id,
                object_type: ObjectTypeKind::None,
            },
        );

        assert_eq!(housing_raw_entry_index_from_event_arg(event_arg), 5);
    }

    #[test]
    fn housing_placard_location_from_packed_subdivision_entry_uses_division_and_plot() {
        let event_arg = u32::from_le_bytes([0, 35, 0, 0]);

        let location = housing_placard_location_from_event_arg(event_arg, 0);

        assert_eq!(location.division, 1);
        assert_eq!(location.plot_index, 5);
    }

    #[test]
    fn housing_placard_location_from_low_byte_plot_uses_fallback_division() {
        let location = housing_placard_location_from_event_arg(5, 1);

        assert_eq!(location.division, 1);
        assert_eq!(location.plot_index, 5);
    }
}
