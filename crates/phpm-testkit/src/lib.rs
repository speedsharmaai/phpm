//! Byte-exact comparison helpers.
//!
//! Anything phpm writes that Composer also writes is compared as bytes, never
//! through a snapshot tool that might normalise whitespace.

/// Panics with the first differing offset and a short window around it.
#[track_caller]
pub fn assert_bytes_eq(actual: &[u8], expected: &[u8]) {
    if actual == expected {
        return;
    }
    let offset = first_difference(actual, expected);
    let window = |bytes: &[u8]| {
        let start = offset.saturating_sub(20);
        let end = (offset + 20).min(bytes.len());
        String::from_utf8_lossy(bytes.get(start..end).unwrap_or_default()).into_owned()
    };
    panic!(
        "bytes differ at offset {offset} (actual {} bytes, expected {} bytes)\n  actual:   {:?}\n  expected: {:?}",
        actual.len(),
        expected.len(),
        window(actual),
        window(expected),
    );
}

/// Index of the first byte where the slices differ, or the shorter length.
#[must_use]
pub fn first_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()))
}

#[cfg(test)]
mod tests {
    use super::{assert_bytes_eq, first_difference};

    #[test]
    fn equal_slices_pass() {
        assert_bytes_eq(b"<?php\n", b"<?php\n");
    }

    #[test]
    fn finds_first_difference() {
        assert_eq!(first_difference(b"abcd", b"abxd"), 2);
        assert_eq!(first_difference(b"abc", b"abcd"), 3);
    }

    #[test]
    #[should_panic(expected = "bytes differ at offset 7")]
    fn trailing_whitespace_is_a_difference() {
        assert_bytes_eq(b"array ( \n", b"array (\n");
    }
}
