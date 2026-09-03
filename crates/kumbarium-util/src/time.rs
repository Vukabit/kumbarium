//! Strict ISO-8601 UTC timestamp parse + format (ms precision).

use std::time::{SystemTime, UNIX_EPOCH};

const MS_PER_DAY: i64 = 86_400_000;

/// Parse a strict ISO-8601 UTC timestamp into Unix epoch ms.
/// Accepts `YYYY-MM-DDTHH:MM:SS[.fff...]Z` only; Z-suffix required,
/// fractional seconds optional and truncated to ms. Returns None on
/// ANY deviation (missing Z, a timezone offset, out-of-range field,
/// junk).
pub fn parse_iso8601_ms(input: &str) -> Option<i64> {
  let b = input.as_bytes();

  // Minimum canonical length is 20: YYYY-MM-DDTHH:MM:SSZ
  if b.len() < 20 {
    return None;
  }

  // Fixed separators; anything else (lowercase t, offset) fails.
  if b[4] != b'-'
    || b[7] != b'-'
    || b[10] != b'T'
    || b[13] != b':'
    || b[16] != b':'
  {
    return None;
  }
  let f = |lo: usize, hi: usize| -> Option<i64> {
    let s = input.get(lo..hi)?;
    if s.bytes().all(|c| c.is_ascii_digit()) {
      s.parse::<i64>().ok()
    } else {
      None
    }
  };
  let year: i64 = f(0, 4)?;
  let month: i64 = f(5, 7)?;
  let day: i64 = f(8, 10)?;
  let hour: i64 = f(11, 13)?;
  let minute: i64 = f(14, 16)?;
  let second: i64 = f(17, 19)?;

  if !(1..=12).contains(&month)
    || !(0..=23).contains(&hour)
    || !(0..=59).contains(&minute)
    || !(0..=59).contains(&second)
    || !(1..=days_in_month(year, month)).contains(&day)
  {
    return None;
  }

  // Fractional seconds + the mandatory trailing Z.
  let ms = match b[19] {
    b'Z' if b.len() == 20 => 0,
    b'.' => {
      let rest = input.get(20..)?;
      let frac = rest.strip_suffix('Z')?;
      if frac.is_empty() || !frac.bytes().all(|c| c.is_ascii_digit()) {
        return None;
      }

      // Truncate to ms: first 3 digits, right padded.
      let mut millisec = 0i64;
      for i in 0..3 {
        millisec = millisec * 10
          + frac.as_bytes().get(i).map_or(0, |c| (c - b'0') as i64);
      }
      millisec
    }
    _ => {
      return None;
    }
  };

  let days = days_from_civil(year, month, day);
  Some(
    days * MS_PER_DAY + hour * 3_600_000 + minute * 60_000 + second * 1000 + ms,
  )
}

/// Format Unix epoch ms as `YYYY-MM-DDTHH:MM:SS.fffZ`. Total for
/// the representable range.
pub fn format_iso8601_ms(unix_ms: i64) -> String {
  // Floor-division so pre-1970 (negative) ms split correctly.
  let days = unix_ms.div_euclid(MS_PER_DAY);
  let tod = unix_ms.rem_euclid(MS_PER_DAY); // [0, 86_399_999]
  let (y, m, d) = civil_from_days(days);
  let h = tod / 3_600_000;
  let min = (tod % 3_600_000) / 60_000;
  let s = (tod % 60_000) / 1000;
  let ms = tod % 1000;

  // Canonical for 0000-9999; extreme inputs widen the year field
  // (total, never panics).
  format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{ms:03}Z")
}

/// The current wall-clock time as Unix epoch milliseconds (UTC). Reads
/// the system clock. A clock set before 1970 yields a negative value,
/// which `format_iso8601_ms` still renders correctly. Note: this is
/// WALL-CLOCK time (for a timestamp), not a monotonic clock; use
/// `std::time::Instant` to measure elapsed durations.
pub fn now_ms() -> i64 {
  // Saturating conversions: an absurdly mis-set clock (millis beyond
  // i64's range) clamps instead of wrapping, so the result is always a
  // sane epoch-ms, never garbage.
  match SystemTime::now().duration_since(UNIX_EPOCH) {
    Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
    // Clock set before the Unix epoch: report how far before, negative.
    Err(e) => i64::try_from(e.duration().as_millis())
      .map(|ms| -ms)
      .unwrap_or(i64::MIN),
  }
}

/// The current wall-clock time as a strict ISO-8601 UTC timestamp
/// (millisecond precision), e.g. `2026-08-05T14:03:21.512Z`. The
/// canonical wall-clock read-point for timestamped records.
pub fn now_iso8601() -> String {
  format_iso8601_ms(now_ms())
}

// Internals: proleptic-Gregorian civil-date arithmetic.

// Days since 1970-01-01 for a proleptic-Gregorian civil date.
// Howard Hinnant's algorithm (public domain). Valid for any
// in-range y/m/d; callers validate ranges before calling.
// Constants: 146097 = days per 400-year era; 719468 = days from the
// algorithm's internal 0000-03-01 epoch to the Unix 1970-01-01 epoch.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
  let y = if m <= 2 { y - 1 } else { y };
  let era = if y >= 0 { y } else { y - 399 } / 400;
  let yoe = y - era * 400;
  let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  era * 146097 + doe - 719468
}

// Inverse: civil (year, month, day) from days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
  let z = z + 719468;
  let era = if z >= 0 { z } else { z - 146096 } / 146097;
  let doe = z - era * 146097;
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
  let y = yoe + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
  (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
  match m {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    2 => {
      if is_leap(y) {
        29
      } else {
        28
      }
    }
    _ => {
      0 // invalid month; the caller's range check rejects it
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn format_anchor_at_epoch() {
    assert_eq!(format_iso8601_ms(0), "1970-01-01T00:00:00.000Z");
  }

  #[test]
  fn round_trips_including_pre_epoch() {
    // 0, a known post-epoch instant, and a NEGATIVE (pre-1970) ms.
    for ms in [0i64, 1_753_939_200_123, -1, -62_135_596_800_000] {
      let s = format_iso8601_ms(ms);
      assert_eq!(parse_iso8601_ms(&s), Some(ms), "round-trip {ms}");
    }
  }

  #[test]
  fn parse_format_round_trips_canonical_strings() {
    for s in [
      "1970-01-01T00:00:00.000Z",
      "2026-07-31T13:45:07.500Z",
      "1969-12-31T23:59:59.999Z",
    ] {
      let ms = parse_iso8601_ms(s).expect("parses");
      assert_eq!(format_iso8601_ms(ms), s);
    }
  }

  #[test]
  fn fractional_seconds_truncate_not_round() {
    let base = "1970-01-01T00:00:00";
    // 1, 2, 3 digits all mean 500 ms.
    for frac in [".5Z", ".50Z", ".500Z"] {
      assert_eq!(parse_iso8601_ms(&format!("{base}{frac}")), Some(500));
    }
    // 4+ digits truncate (500), never round up to 501.
    assert_eq!(parse_iso8601_ms(&format!("{base}.5009Z")), Some(500));
  }

  #[test]
  fn leap_year_matrix() {
    assert!(parse_iso8601_ms("2000-02-29T00:00:00Z").is_some());
    assert!(parse_iso8601_ms("2020-02-29T00:00:00Z").is_some());
    // 1900 is a century non-leap year; 2021 is not a leap year.
    assert!(parse_iso8601_ms("1900-02-29T00:00:00Z").is_none());
    assert!(parse_iso8601_ms("2021-02-29T00:00:00Z").is_none());
  }

  #[test]
  fn strict_rejection_table() {
    let bad = [
      "2026-07-31T00:00:00",       // missing Z
      "2026-07-31T00:00:00z",      // lowercase z
      "2026-07-31t00:00:00Z",      // lowercase t
      "2026-07-31T00:00:00+00:00", // timezone offset
      "26-07-31T00:00:00Z",        // 2-digit year
      "2026-13-01T00:00:00Z",      // month 13
      "2026-07-00T00:00:00Z",      // day 00
      "2026-07-32T00:00:00Z",      // day 32
      "2026-07-31T24:00:00Z",      // hour 24
      "2026-07-31T00:60:00Z",      // minute 60
      "2026-07-31T00:00:60Z",      // second 60 (no leap seconds)
      "2026-07-31T00:00:00.500ZZ", // trailing garbage
      "",                          // empty
    ];
    for s in bad {
      assert!(parse_iso8601_ms(s).is_none(), "should reject {s:?}");
    }
  }

  #[test]
  fn now_is_plausible_and_round_trips() {
    // 2020-01-01T00:00:00Z in epoch ms: a sane floor for a real clock.
    const Y2020_MS: i64 = 1_577_836_800_000;
    assert!(now_ms() > Y2020_MS, "clock before 2020?");
    let s = now_iso8601();
    assert!(s.ends_with('Z'), "not a UTC stamp: {s}");
    assert!(parse_iso8601_ms(&s).is_some_and(|ms| ms > Y2020_MS));
  }
}

#[cfg(test)]
mod prop_tests {
  use super::*;
  use proptest::prelude::*;

  proptest! {
    // Round-trip across the full 0001-01-01 .. 9999-12-31 range.
    #[test]
    fn round_trips_across_the_year_range(
      ms in -62_135_596_800_000i64..=253_402_300_799_999i64
    ) {
      let s = format_iso8601_ms(ms);
      prop_assert_eq!(parse_iso8601_ms(&s), Some(ms));
    }

    // Panic-freedom on arbitrary input (the always-on layer; the
    // coverage-guided libfuzzer target lives in fuzz/).
    #[test]
    fn parse_never_panics(s in ".*") {
      let _ = parse_iso8601_ms(&s);
    }

    // Denser near the grammar alphabet.
    #[test]
    fn parse_never_panics_near_grammar(s in "[0-9:.TZ+-]{0,30}") {
      let _ = parse_iso8601_ms(&s);
    }
  }
}
