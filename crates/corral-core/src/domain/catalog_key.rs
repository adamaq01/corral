use super::is_slug;
use crate::{Error, Result};

validated_string!(ImageKey, check);
validated_string!(SizeKey, check);

fn check(value: &str) -> Result<()> {
    if is_slug(value) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "catalog key {value:?} must match [a-z0-9-]{{1,32}}"
        )))
    }
}
