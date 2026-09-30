use std::cmp::Ordering;

/// PHP `strnatcmp`, the comparison behind `sort($a, SORT_NATURAL)`.
pub fn strnatcmp(a: &str, b: &str) -> Ordering {
    natural(a.as_bytes(), b.as_bytes(), false)
}

/// PHP `strnatcasecmp`.
pub fn strnatcasecmp(a: &str, b: &str) -> Ordering {
    natural(a.as_bytes(), b.as_bytes(), true)
}

fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

fn is_digit_at(s: &[u8], i: usize) -> bool {
    i < s.len() && s[i].is_ascii_digit()
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

// php-src: ext/standard/strnatcmp.c strnatcmp_ex
fn natural(a: &[u8], b: &[u8], fold_case: bool) -> Ordering {
    if a.is_empty() || b.is_empty() {
        return a.len().cmp(&b.len());
    }
    let (mut ai, mut bi) = (0, 0);
    let mut leading = true;
    loop {
        let mut ca = at(a, ai);
        let mut cb = at(b, bi);

        while leading && ca == b'0' && ai + 1 < a.len() && a[ai + 1].is_ascii_digit() {
            ai += 1;
            ca = a[ai];
        }
        while leading && cb == b'0' && bi + 1 < b.len() && b[bi + 1].is_ascii_digit() {
            bi += 1;
            cb = b[bi];
        }
        leading = false;

        while is_space(ca) {
            ai += 1;
            ca = at(a, ai);
        }
        while is_space(cb) {
            bi += 1;
            cb = at(b, bi);
        }

        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let result = if ca == b'0' || cb == b'0' {
                compare_left(a, &mut ai, b, &mut bi)
            } else {
                compare_right(a, &mut ai, b, &mut bi)
            };
            if result != Ordering::Equal {
                return result;
            }
            match (ai == a.len(), bi == b.len()) {
                (true, true) => return Ordering::Equal,
                (true, false) => return Ordering::Less,
                (false, true) => return Ordering::Greater,
                (false, false) => {
                    ca = at(a, ai);
                    cb = at(b, bi);
                }
            }
        }

        if fold_case {
            ca = ca.to_ascii_uppercase();
            cb = cb.to_ascii_uppercase();
        }
        match ca.cmp(&cb) {
            Ordering::Equal => {}
            other => return other,
        }

        ai += 1;
        bi += 1;
        match (ai >= a.len(), bi >= b.len()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }
    }
}

fn compare_right(a: &[u8], ai: &mut usize, b: &[u8], bi: &mut usize) -> Ordering {
    let mut bias = Ordering::Equal;
    loop {
        match (is_digit_at(a, *ai), is_digit_at(b, *bi)) {
            (false, false) => return bias,
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            (true, true) => {
                if bias == Ordering::Equal {
                    bias = a[*ai].cmp(&b[*bi]);
                }
            }
        }
        *ai += 1;
        *bi += 1;
    }
}

fn compare_left(a: &[u8], ai: &mut usize, b: &[u8], bi: &mut usize) -> Ordering {
    loop {
        match (is_digit_at(a, *ai), is_digit_at(b, *bi)) {
            (false, false) => return Ordering::Equal,
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            (true, true) => match a[*ai].cmp(&b[*bi]) {
                Ordering::Equal => {}
                other => return other,
            },
        }
        *ai += 1;
        *bi += 1;
    }
}

/// How PHP 8 compares two strings under `SORT_REGULAR` (`sort`, `ksort`):
/// numerically when both are numeric strings, bytewise otherwise.
pub fn smart_strcmp(a: &str, b: &str) -> Ordering {
    match (numeric(a), numeric(b)) {
        (Some(Numeric::Int(x)), Some(Numeric::Int(y))) => x.cmp(&y),
        (Some(x), Some(y)) => x.as_f64().total_cmp(&y.as_f64()),
        _ => a.as_bytes().cmp(b.as_bytes()),
    }
}

#[derive(Debug, Clone, Copy)]
enum Numeric {
    Int(i64),
    Float(f64),
}

impl Numeric {
    #[expect(clippy::cast_precision_loss, reason = "PHP converts the same way")]
    fn as_f64(self) -> f64 {
        match self {
            Self::Int(i) => i as f64,
            Self::Float(f) => f,
        }
    }
}

// php-src: Zend/zend_operators.c _is_numeric_string_ex (no trailing data allowed)
fn numeric(s: &str) -> Option<Numeric> {
    let t = s.trim_matches(|c: char| c.is_ascii() && is_space(c as u8));
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let bytes = body.as_bytes();
    let int_digits = bytes.iter().take_while(|c| c.is_ascii_digit()).count();
    let mut i = int_digits;
    let mut frac_digits = 0;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        frac_digits = bytes[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        i += frac_digits;
    }
    if int_digits + frac_digits == 0 {
        return None;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp_digits = bytes[j..].iter().take_while(|c| c.is_ascii_digit()).count();
        if exp_digits > 0 {
            i = j + exp_digits;
        }
    }
    if i != bytes.len() {
        return None;
    }
    if i == int_digits
        && let Ok(n) = t.parse::<i64>()
    {
        return Some(Numeric::Int(n));
    }
    t.parse::<f64>().ok().map(Numeric::Float)
}

#[cfg(test)]
mod tests {
    use super::{smart_strcmp, strnatcasecmp, strnatcmp};
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn natural_order_matches_php() {
        let cases = [
            ("img12", "img10", Greater),
            ("img2", "img10", Less),
            ("1.10", "1.9", Greater),
            ("9999999-dev", "2.x-dev", Greater),
            ("a", "", Greater),
            ("", "", Equal),
            ("abc", "abc", Equal),
            ("abc", "abcd", Less),
            ("x01", "x1", Less),
            ("x001", "x01", Less),
            ("007", "7", Equal),
            ("0a", "a", Less),
            ("a 1", "a1", Equal),
            ("a  ", "a", Greater),
            ("A", "a", Less),
            ("1.0", "1.0.0", Less),
            ("2", "10", Less),
            ("0.5", "0.10", Less),
        ];
        for (a, b, expected) in cases {
            assert_eq!(strnatcmp(a, b), expected, "strnatcmp({a:?}, {b:?})");
        }
    }

    #[test]
    fn case_insensitive_natural_order() {
        assert_eq!(strnatcasecmp("A", "a"), Equal);
        assert_eq!(strnatcasecmp("Foo2", "foo10"), Less);
        assert_eq!(strnatcasecmp("b", "A"), Greater);
    }

    #[test]
    fn smart_comparison_is_numeric_only_for_numeric_pairs() {
        assert_eq!(smart_strcmp("10", "9"), Greater);
        assert_eq!(smart_strcmp("10", "9a"), Less);
        assert_eq!(smart_strcmp("1e3", "999"), Greater);
        assert_eq!(smart_strcmp("1.0", "1"), Equal);
        assert_eq!(smart_strcmp(" 1", "1 "), Equal);
        assert_eq!(smart_strcmp("-5", "3"), Less);
        assert_eq!(smart_strcmp(".5", "0.4"), Greater);
        assert_eq!(smart_strcmp("abc", "abx"), Less);
        assert_eq!(smart_strcmp("1e", "1"), Greater);
        assert_eq!(smart_strcmp(".", "1"), Less);
        assert_eq!(smart_strcmp("99999999999999999999", "1"), Greater);
    }
}
