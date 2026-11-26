use std::num::NonZeroU64;

use crate::{Error, Result};

const BINARY_SUFFIXES: [(&str, u64); 6] = [
    ("Ei", 1 << 60),
    ("Pi", 1 << 50),
    ("Ti", 1 << 40),
    ("Gi", 1 << 30),
    ("Mi", 1 << 20),
    ("Ki", 1 << 10),
];

const DECIMAL_SUFFIXES: [(&str, u64); 6] = [
    ("E", 1_000_000_000_000_000_000),
    ("P", 1_000_000_000_000_000),
    ("T", 1_000_000_000_000),
    ("G", 1_000_000_000),
    ("M", 1_000_000),
    ("k", 1_000),
];

/// A Kubernetes-style CPU quantity such as `2`, `1.5` or `500m`, in thousandths of a CPU.
pub fn cpu_millis(text: &str) -> Result<NonZeroU64> {
    let parsed = match text.strip_suffix('m') {
        Some(millis) => scaled(millis, 1),
        None => scaled(text, 1_000),
    };
    parsed.ok_or_else(|| invalid("CPU", text))
}

/// A Kubernetes-style byte quantity such as `4Gi`, `512Mi` or `1G`.
pub fn bytes(text: &str) -> Result<NonZeroU64> {
    let (number, multiplier) = BINARY_SUFFIXES
        .iter()
        .chain(&DECIMAL_SUFFIXES)
        .find_map(|(suffix, multiplier)| {
            text.strip_suffix(suffix)
                .map(|number| (number, *multiplier))
        })
        .unwrap_or((text, 1));
    scaled(number, multiplier).ok_or_else(|| invalid("byte", text))
}

#[must_use]
pub fn format_cpu(millis: u64) -> String {
    if millis.is_multiple_of(1_000) {
        (millis / 1_000).to_string()
    } else {
        format!("{millis}m")
    }
}

#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    BINARY_SUFFIXES
        .iter()
        .find(|(_, unit)| bytes.is_multiple_of(*unit))
        .map_or_else(
            || bytes.to_string(),
            |(suffix, unit)| format!("{}{suffix}", bytes / unit),
        )
}

fn scaled(number: &str, multiplier: u64) -> Option<NonZeroU64> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let divisor = 10_u128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let value = digits
        .parse::<u128>()
        .ok()?
        .checked_mul(u128::from(multiplier))?
        .div_ceil(divisor);
    // Docker and Kubernetes hold quantities as signed 64-bit integers.
    let value = i64::try_from(value).ok()?;
    NonZeroU64::new(u64::try_from(value).ok()?)
}

fn invalid(kind: &str, text: &str) -> Error {
    Error::Invalid(format!(
        "{kind} quantity {text:?} must be a positive number with an optional Kubernetes suffix"
    ))
}

#[cfg(test)]
mod tests {
    use super::{bytes, cpu_millis, format_bytes, format_cpu};
    use crate::Error;

    #[test]
    fn cpu_quantities_parse_and_format() {
        for (text, millis) in [
            ("2", 2000),
            ("1.5", 1500),
            ("500m", 500),
            ("0.25", 250),
            ("1m", 1),
        ] {
            assert_eq!(cpu_millis(text).unwrap().get(), millis, "{text}");
        }
        for bad in ["", "0", "m", "-1", "two", "1.2.3", "1Gi", " 1"] {
            assert!(matches!(cpu_millis(bad), Err(Error::Invalid(_))), "{bad:?}");
        }
        assert_eq!(
            (format_cpu(2000), format_cpu(500), format_cpu(1500)),
            ("2".into(), "500m".into(), "1500m".into())
        );
    }

    #[test]
    fn byte_quantities_parse_and_format() {
        for (text, value) in [
            ("4Gi", 4 << 30),
            ("512Mi", 512 << 20),
            ("1.5Gi", 1_610_612_736),
            ("1G", 1_000_000_000),
            ("64k", 64_000),
            ("1048576", 1 << 20),
        ] {
            assert_eq!(bytes(text).unwrap().get(), value, "{text}");
        }
        for bad in ["", "0", "Gi", "4GB", "4 Gi", "-1Gi", "9Ei"] {
            assert!(matches!(bytes(bad), Err(Error::Invalid(_))), "{bad:?}");
        }
        assert_eq!(format_bytes(4 << 30), "4Gi");
        assert_eq!(format_bytes(1_610_612_736), "1536Mi");
        assert_eq!(format_bytes(1_000_000_000), "1000000000");
        for value in [1u64, 1 << 10, 3 << 20, 7 << 30, 1_610_612_736] {
            assert_eq!(bytes(&format_bytes(value)).unwrap().get(), value, "{value}");
        }
    }
}
