//! Minimal RFC 3339 timestamp parsing (provider observation times carry offsets).

/// Parse `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` to UTC milliseconds.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b't' || b[10] == b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut i = 19;
    let mut ms = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = s.get(start..i)?;
        if frac.is_empty() {
            return None;
        }
        let padded = format!("{:0<3}", &frac[..frac.len().min(3)]);
        ms = padded.parse().ok()?;
    }
    let offset_min = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        b'+' | b'-' if i + 6 == b.len() && b[i + 3] == b':' => {
            let sign = if b[i] == b'-' { -1 } else { 1 };
            sign * (num(i + 1..i + 3)? * 60 + num(i + 4..i + 6)?)
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some(((days * 86_400 + h * 3600 + mi * 60 + se - offset_min * 60) * 1000) + ms)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_offsets_and_fractions() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("2026-10-07T22:15:00+00:00"), Some(1_791_411_300_000));
        // Same instant expressed with a negative offset.
        assert_eq!(parse_rfc3339_ms("2026-10-07T16:15:00-06:00"), Some(1_791_411_300_000));
        assert_eq!(parse_rfc3339_ms("2026-10-07T22:15:00.25Z"), Some(1_791_411_300_250));
        assert_eq!(parse_rfc3339_ms("2024-02-29T12:00:00Z"), Some(1_709_208_000_000));
    }

    #[test]
    fn rejects_malformed() {
        for s in [
            "",
            "2026-10-07",
            "2026-13-07T00:00:00Z",
            "2026-10-07T25:00:00Z",
            "2026-10-07T00:00:00",
            "2026-10-07T00:00:00+0600",
            "garbage-here-and-there",
        ] {
            assert_eq!(parse_rfc3339_ms(s), None, "{s}");
        }
    }
}
