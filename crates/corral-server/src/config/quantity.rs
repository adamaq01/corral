use anyhow::{Result, anyhow};

const MEMORY_SUFFIXES: [(&str, u128); 12] = [
    ("Ki", 1 << 10),
    ("Mi", 1 << 20),
    ("Gi", 1 << 30),
    ("Ti", 1 << 40),
    ("Pi", 1 << 50),
    ("Ei", 1 << 60),
    ("k", 1_000),
    ("M", 1_000_000),
    ("G", 1_000_000_000),
    ("T", 1_000_000_000_000),
    ("P", 1_000_000_000_000_000),
    ("E", 1_000_000_000_000_000_000),
];

/// A Kubernetes-style CPU quantity such as `2`, `1.5` or `500m`, in thousandths of a CPU.
pub(super) fn cpu_millis(quantity: &str) -> Result<u64> {
    let parsed = match quantity.strip_suffix('m') {
        Some(millis) => scaled(millis, 1),
        None => scaled(quantity, 1_000),
    };
    parsed.ok_or_else(|| invalid("CPU", quantity))
}

/// A Kubernetes-style byte quantity such as `4Gi`, `512Mi` or `1G`.
pub(super) fn bytes(quantity: &str) -> Result<u64> {
    let (number, multiplier) = MEMORY_SUFFIXES
        .iter()
        .find_map(|(suffix, multiplier)| {
            quantity
                .strip_suffix(suffix)
                .map(|number| (number, *multiplier))
        })
        .unwrap_or((quantity, 1));
    scaled(number, multiplier).ok_or_else(|| invalid("byte", quantity))
}

fn scaled(number: &str, multiplier: u128) -> Option<u64> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let divisor = 10_u128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let value = digits
        .parse::<u128>()
        .ok()?
        .checked_mul(multiplier)?
        .div_ceil(divisor);
    // Docker and Kubernetes hold quantities as signed 64-bit integers.
    let value = i64::try_from(value).ok().filter(|value| *value > 0)?;
    u64::try_from(value).ok()
}

fn invalid(kind: &str, quantity: &str) -> anyhow::Error {
    anyhow!(
        "{kind} quantity {quantity:?} must be a positive number with an optional Kubernetes suffix"
    )
}

#[cfg(test)]
mod tests {
    use super::{bytes, cpu_millis};

    #[test]
    fn cpu_quantities() {
        for (quantity, expected) in [
            ("2", 2000),
            ("1.5", 1500),
            ("500m", 500),
            ("0.25", 250),
            ("1m", 1),
        ] {
            assert_eq!(cpu_millis(quantity).unwrap(), expected, "{quantity}");
        }
        for bad in ["", "0", "m", "-1", "two", "1.2.3", "1Gi", " 1"] {
            assert!(cpu_millis(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn byte_quantities() {
        for (quantity, expected) in [
            ("4Gi", 4 * 1024 * 1024 * 1024),
            ("512Mi", 512 * 1024 * 1024),
            ("1.5Gi", 1_610_612_736),
            ("1G", 1_000_000_000),
            ("64k", 64_000),
            ("1048576", 1_048_576),
        ] {
            assert_eq!(bytes(quantity).unwrap(), expected, "{quantity}");
        }
        for bad in ["", "0", "Gi", "4GB", "4 Gi", "-1Gi", "9Ei"] {
            assert!(bytes(bad).is_err(), "{bad:?}");
        }
    }
}
