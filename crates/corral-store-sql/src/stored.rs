use corral_core::{Error, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(crate) fn parsed<T, E>(column: &str, value: Result<T, E>) -> Result<T>
where
    E: std::error::Error + Send + Sync + 'static,
{
    value.map_err(|err| Error::internal(format!("stored {column}"), err))
}

pub(crate) fn variant_name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => unreachable!("stored enums serialize as their variant name"),
    }
}

pub(crate) fn variant<T: DeserializeOwned>(column: &str, name: String) -> Result<T> {
    parsed(column, serde_json::from_value(Value::String(name)))
}
