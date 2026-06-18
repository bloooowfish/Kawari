pub fn truncate_utf8_to_raw_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }

    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }

    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_utf8_truncation_never_splits_codepoint() {
        let value = "abcé😀z";

        for max_bytes in 0..=value.len() {
            let truncated = truncate_utf8_to_raw_bytes(value, max_bytes);
            assert!(truncated.len() <= max_bytes);
            assert!(value.starts_with(&truncated));
            assert!(truncated.is_char_boundary(truncated.len()));
        }

        assert_eq!(truncate_utf8_to_raw_bytes("éé", 3), "é");
        assert_eq!(truncate_utf8_to_raw_bytes("é", 1), "");
    }
}
