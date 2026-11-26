use std::num::NonZeroU64;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

pub(super) fn cpu_millis<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<NonZeroU64, D::Error> {
    corral_core::domain::cpu_millis(&String::deserialize(deserializer)?).map_err(D::Error::custom)
}

pub(super) fn bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NonZeroU64, D::Error> {
    corral_core::domain::bytes(&String::deserialize(deserializer)?).map_err(D::Error::custom)
}
