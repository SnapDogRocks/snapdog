// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Command payload parsing.
//!
//! One rule for every command: a payload is either understood exactly or
//! rejected. A payload that is not understood changes nothing, and the error
//! never contains the payload (it may come from anywhere on the network, and
//! payloads are not logged above debug level).

use snapdog_common::RepeatMode;

/// Largest payload any command accepts, in bytes. The longest legitimate one is a
/// decimal number or a word like `previous`.
pub const MAX_PAYLOAD_BYTES: usize = 256;
/// Seeking beyond a week is a typo, not a position.
const MAX_SEEK_SECS: f64 = 7.0 * 24.0 * 3600.0;
/// Snapcast latency offsets beyond ten seconds are a typo, not a setting.
const MAX_LATENCY_MS: i32 = 10_000;

/// A payload that could not be understood. The text names the expectation, never
/// the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayloadError(pub &'static str);

impl std::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for PayloadError {}

type Parsed<T> = Result<T, PayloadError>;

/// `true`/`on`/`1` or `false`/`off`/`0`, case-insensitive, surrounding whitespace
/// ignored. Home Assistant's switches send `ON` and `OFF` by default.
///
/// # Errors
///
/// Anything else is an error: a garbled mute command must not unmute.
pub fn parse_bool(payload: &str) -> Parsed<bool> {
    match payload.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "1" => Ok(true),
        "false" | "off" | "0" => Ok(false),
        _ => Err(PayloadError("expected true/false, on/off or 1/0")),
    }
}

/// Zone volume as a percentage in `0..=100`.
///
/// Two scales are accepted, told apart by the value: **up to and including `1.0`
/// is a fraction** (`0.5` is 50 %, Home Assistant's `volume_level`), **anything
/// above is a percentage** (`50` is 50 %, the original `SnapDog` scale). So `1` is
/// 100 %, not 1 %; send `0.01` for 1 %. Out-of-range values are clamped.
/// Fractions of a percent round, percentages truncate.
///
/// # Errors
///
/// Not a number, or not finite (`NaN`, `inf`).
pub fn parse_zone_volume(payload: &str) -> Parsed<i32> {
    let volume: f64 = payload
        .trim()
        .parse()
        .map_err(|_| PayloadError("volume is not a number"))?;
    if !volume.is_finite() {
        return Err(PayloadError("volume is not a finite number"));
    }
    let percent = if volume <= 1.0 {
        (volume * 100.0).round()
    } else {
        volume.trunc()
    };
    Ok((percent as i32).clamp(0, 100))
}

/// Client volume as an integer percentage, clamped to `0..=100`. No fractions:
/// unlike zones, clients never had the `0.0..=1.0` scale.
///
/// # Errors
///
/// Not an integer.
pub fn parse_client_volume(payload: &str) -> Parsed<i32> {
    let volume: i32 = payload
        .trim()
        .parse()
        .map_err(|_| PayloadError("volume is not an integer"))?;
    Ok(volume.clamp(0, 100))
}

/// A seek position in seconds (fractions allowed) as milliseconds.
///
/// # Errors
///
/// Not a number, negative, not finite or beyond a week.
pub fn parse_seek_ms(payload: &str) -> Parsed<i64> {
    let secs: f64 = payload
        .trim()
        .parse()
        .map_err(|_| PayloadError("position is not a number"))?;
    if !secs.is_finite() || !(0.0..=MAX_SEEK_SECS).contains(&secs) {
        return Err(PayloadError("position is out of range"));
    }
    Ok((secs * 1000.0) as i64)
}

/// `off`, `one`/`track`, `all`/`playlist`, case-insensitive.
///
/// # Errors
///
/// Anything else (it used to mean `off`).
pub fn parse_repeat(payload: &str) -> Parsed<RepeatMode> {
    match payload.trim().to_ascii_lowercase().as_str() {
        "off" => Ok(RepeatMode::Off),
        "one" | "track" => Ok(RepeatMode::Track),
        "all" | "playlist" => Ok(RepeatMode::Playlist),
        _ => Err(PayloadError("expected off, one/track or all/playlist")),
    }
}

/// Transport commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
}

/// `play`, `pause`, `stop`, `next`, `previous`, case-insensitive.
///
/// # Errors
///
/// Anything else.
pub fn parse_transport(payload: &str) -> Parsed<Transport> {
    match payload.trim().to_ascii_lowercase().as_str() {
        "play" => Ok(Transport::Play),
        "pause" => Ok(Transport::Pause),
        "stop" => Ok(Transport::Stop),
        "next" => Ok(Transport::Next),
        "previous" => Ok(Transport::Previous),
        _ => Err(PayloadError("unknown control command")),
    }
}

/// A non-negative integer made of ASCII digits only (no sign, no fraction).
///
/// # Errors
///
/// Anything else, or a value that does not fit `u32`.
pub fn parse_count(payload: &str) -> Parsed<usize> {
    let digits = payload.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PayloadError("expected a non-negative integer"));
    }
    digits
        .parse::<u32>()
        .map(|n| n as usize)
        .map_err(|_| PayloadError("integer is out of range"))
}

/// Seconds as `u16`.
///
/// # Errors
///
/// Not a non-negative integer up to 65535.
pub fn parse_seconds(payload: &str) -> Parsed<u16> {
    let n = parse_count(payload)?;
    u16::try_from(n).map_err(|_| PayloadError("seconds are out of range (0..=65535)"))
}

/// Client latency in milliseconds, within ±10 s.
///
/// # Errors
///
/// Not an integer, or out of range.
pub fn parse_latency_ms(payload: &str) -> Parsed<i32> {
    let ms: i32 = payload
        .trim()
        .parse()
        .map_err(|_| PayloadError("latency is not an integer"))?;
    if ms.abs() > MAX_LATENCY_MS {
        return Err(PayloadError("latency is out of range (±10000 ms)"));
    }
    Ok(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_accepts_the_documented_spellings_in_any_case() {
        for yes in [
            "true", "TRUE", "True", "on", "ON", "On", "1", " true\n", "\ton ",
        ] {
            assert_eq!(parse_bool(yes), Ok(true), "{yes:?}");
        }
        for no in ["false", "FALSE", "off", "OFF", "0", " off \r\n"] {
            assert_eq!(parse_bool(no), Ok(false), "{no:?}");
        }
    }

    #[test]
    fn bool_rejects_everything_else_instead_of_guessing() {
        for bad in [
            "",
            " ",
            "yes",
            "no",
            "2",
            "-1",
            "tru",
            "toggle",
            "true false",
            "1.0",
        ] {
            assert!(parse_bool(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn zone_volume_takes_fractions_up_to_one_and_percentages_above() {
        for (payload, percent) in [
            ("0", 0),
            ("0.0", 0),
            ("0.5", 50),
            ("0.505", 51), // fractions round
            ("0.994", 99),
            ("0.995", 100),
            ("1.0", 100), // 1 is the top of the fraction scale, i.e. 100 %
            ("1", 100),
            ("1.5", 1), // above 1.0 it is a percentage, truncated
            ("2", 2),
            ("50", 50),
            ("50.9", 50),
            ("100", 100),
            ("100.0", 100),
            (" 75\n", 75),
            ("1e2", 100),
        ] {
            assert_eq!(parse_zone_volume(payload), Ok(percent), "{payload:?}");
        }
    }

    #[test]
    fn zone_volume_clamps_out_of_range_values() {
        for (payload, percent) in [
            ("101", 100),
            ("1000", 100),
            ("1e300", 100),
            ("-1", 0),
            ("-0.5", 0),
            ("-100", 0),
        ] {
            assert_eq!(parse_zone_volume(payload), Ok(percent), "{payload:?}");
        }
    }

    #[test]
    fn zone_volume_rejects_non_numbers_and_non_finite_numbers() {
        // `NaN` parses as an f64 and used to become volume 0.
        for bad in [
            "", "loud", "NaN", "nan", "inf", "-inf", "infinity", "50%", "1,5",
        ] {
            assert!(parse_zone_volume(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn client_volume_is_an_integer_percentage_clamped() {
        assert_eq!(parse_client_volume("80"), Ok(80));
        assert_eq!(parse_client_volume(" 80 "), Ok(80));
        assert_eq!(parse_client_volume("200"), Ok(100));
        assert_eq!(parse_client_volume("-5"), Ok(0));
        for bad in ["", "0.5", "50.0", "x", "NaN"] {
            assert!(parse_client_volume(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn seek_converts_seconds_to_milliseconds_within_bounds() {
        assert_eq!(parse_seek_ms("30"), Ok(30_000));
        assert_eq!(parse_seek_ms("0"), Ok(0));
        assert_eq!(parse_seek_ms("12.5"), Ok(12_500));
        assert_eq!(parse_seek_ms("604800"), Ok(604_800_000));
        for bad in ["-1", "-0.001", "604801", "NaN", "inf", "", "x", "1e300"] {
            assert!(parse_seek_ms(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn repeat_has_three_modes_and_no_default() {
        assert_eq!(parse_repeat("off"), Ok(RepeatMode::Off));
        assert_eq!(parse_repeat("one"), Ok(RepeatMode::Track));
        assert_eq!(parse_repeat("TRACK"), Ok(RepeatMode::Track));
        assert_eq!(parse_repeat("all"), Ok(RepeatMode::Playlist));
        assert_eq!(parse_repeat(" Playlist "), Ok(RepeatMode::Playlist));
        for bad in ["", "none", "false", "1", "repeat", "of"] {
            assert!(parse_repeat(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn transport_commands_are_case_insensitive_and_trimmed() {
        assert_eq!(parse_transport("play"), Ok(Transport::Play));
        assert_eq!(parse_transport("PAUSE"), Ok(Transport::Pause));
        assert_eq!(parse_transport("Stop\n"), Ok(Transport::Stop));
        assert_eq!(parse_transport(" next "), Ok(Transport::Next));
        assert_eq!(parse_transport("previous"), Ok(Transport::Previous));
        for bad in ["", "prev", "skip", "play pause", "toggle"] {
            assert!(parse_transport(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn counts_are_plain_unsigned_integers() {
        assert_eq!(parse_count("3"), Ok(3));
        assert_eq!(parse_count(" 12\n"), Ok(12));
        assert_eq!(parse_count("007"), Ok(7));
        for bad in ["", "-1", "+1", "1.0", "x", "4294967296", "1 2"] {
            assert!(parse_count(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_seconds("65535"), Ok(65_535));
        assert!(parse_seconds("65536").is_err());
    }

    #[test]
    fn latency_is_an_integer_within_ten_seconds() {
        assert_eq!(parse_latency_ms("20"), Ok(20));
        assert_eq!(parse_latency_ms("-20"), Ok(-20));
        assert_eq!(parse_latency_ms("10000"), Ok(10_000));
        for bad in ["10001", "-10001", "2147483647", "1.5", "", "x"] {
            assert!(parse_latency_ms(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn errors_never_contain_the_payload() {
        let secret = "hunter2-secret-payload";
        let errors = [
            parse_bool(secret).unwrap_err(),
            parse_zone_volume(secret).unwrap_err(),
            parse_client_volume(secret).unwrap_err(),
            parse_seek_ms(secret).unwrap_err(),
            parse_repeat(secret).unwrap_err(),
            parse_transport(secret).unwrap_err(),
            parse_count(secret).unwrap_err(),
            parse_latency_ms(secret).unwrap_err(),
        ];
        for e in errors {
            assert!(!e.to_string().contains("hunter2"), "{e}");
        }
    }
}
