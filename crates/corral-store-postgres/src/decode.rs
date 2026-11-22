use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgRow;

pub(crate) fn parsed<T>(
    row: &PgRow,
    column: &str,
    parse: impl FnOnce(&str) -> corral_core::Result<T>,
) -> sqlx::Result<T> {
    parse(row.try_get(column)?).map_err(|err| sqlx::Error::ColumnDecode {
        index: column.to_owned(),
        source: Box::new(err),
    })
}

pub(crate) fn text<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(text)) => text,
        _ => unreachable!("stored enums serialize as strings"),
    }
}

pub(crate) fn from_text<T: DeserializeOwned>(text: &str) -> corral_core::Result<T> {
    serde_json::from_value(Value::String(text.to_owned()))
        .map_err(|err| corral_core::Error::internal("store", err))
}
