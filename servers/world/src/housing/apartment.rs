pub const MAX_APARTMENT_ROOM_NUMBER: u16 = 1023;

pub fn valid_apartment_room_number(room_number: u16) -> bool {
    (1..=MAX_APARTMENT_ROOM_NUMBER).contains(&room_number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apartment_room_number_accepts_packed_house_id_limit() {
        assert!(!valid_apartment_room_number(0));
        assert!(valid_apartment_room_number(1));
        assert!(valid_apartment_room_number(MAX_APARTMENT_ROOM_NUMBER));
        assert!(!valid_apartment_room_number(MAX_APARTMENT_ROOM_NUMBER + 1));
    }
}
