use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{ImageKey, SizeKey};

/// The image and size a user's new sandbox gets when its creation names none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SandboxDefaults {
    pub image: ImageKey,
    pub size: SizeKey,
}
