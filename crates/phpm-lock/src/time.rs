/// A lock `time` as `DateTime::format(DATE_RFC3339)` prints it after
/// `new DateTime($time, new DateTimeZone('UTC'))`, for the formats Composer
/// writes: RFC 3339 with an offset or `Z`, `Y-m-d H:i:s`, or a Unix timestamp.
/// `None` for anything else.
// Composer: Package/Loader/ArrayLoader.php configureObject (time)
pub(crate) fn rfc3339(time: &str) -> Option<String> {
    if !time.is_empty() && time.bytes().all(|b| b.is_ascii_digit()) {
        return time.parse::<i64>().ok().map(from_unix);
    }
    let b = time.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    let shape = b.len() >= 19
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && matches!(b[10], b'T' | b't' | b' ')
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19);
    if !shape {
        return None;
    }
    let mut rest = &time[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        rest = &frac[n..];
    }
    let offset = match rest {
        "" | "Z" | "z" => "+00:00".to_owned(),
        o if o.len() == 6 && matches!(o.as_bytes()[0], b'+' | b'-') && o.as_bytes()[3] == b':' => {
            o.to_owned()
        }
        o if o.len() == 5
            && matches!(o.as_bytes()[0], b'+' | b'-')
            && o[1..].bytes().all(|c| c.is_ascii_digit()) =>
        {
            format!("{}:{}", &o[..3], &o[3..])
        }
        _ => return None,
    };
    Some(format!("{}T{}{offset}", &time[..10], &time[11..19]))
}

fn from_unix(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::rfc3339;

    #[test]
    fn keeps_rfc3339_with_offsets() {
        assert_eq!(
            rfc3339("2025-08-08T20:05:48+00:00").as_deref(),
            Some("2025-08-08T20:05:48+00:00")
        );
        assert_eq!(
            rfc3339("2025-08-08T20:05:48+02:00").as_deref(),
            Some("2025-08-08T20:05:48+02:00")
        );
        assert_eq!(
            rfc3339("2025-08-08T20:05:48-0530").as_deref(),
            Some("2025-08-08T20:05:48-05:30")
        );
    }

    #[test]
    fn assumes_utc_without_zone() {
        assert_eq!(
            rfc3339("2012-01-02 03:04:05").as_deref(),
            Some("2012-01-02T03:04:05+00:00")
        );
        assert_eq!(
            rfc3339("2012-01-02T03:04:05Z").as_deref(),
            Some("2012-01-02T03:04:05+00:00")
        );
        assert_eq!(
            rfc3339("2012-01-02T03:04:05.123456Z").as_deref(),
            Some("2012-01-02T03:04:05+00:00")
        );
    }

    #[test]
    fn converts_unix_timestamps() {
        assert_eq!(rfc3339("0").as_deref(), Some("1970-01-01T00:00:00+00:00"));
        assert_eq!(
            rfc3339("1700000000").as_deref(),
            Some("2023-11-14T22:13:20+00:00")
        );
        assert_eq!(
            rfc3339("951782400").as_deref(),
            Some("2000-02-29T00:00:00+00:00")
        );
    }

    #[test]
    fn rejects_other_formats() {
        assert_eq!(rfc3339("yesterday"), None);
        assert_eq!(rfc3339("2012-01-02T03:04:05."), None);
        assert_eq!(rfc3339("2012-01-02T03:04:05 CET"), None);
    }
}
