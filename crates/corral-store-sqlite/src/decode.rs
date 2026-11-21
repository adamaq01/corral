use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;
use uuid::fmt::Hyphenated;

pub(crate) fn uuid(row: &SqliteRow, column: &str) -> sqlx::Result<Uuid> {
    Ok(row.try_get::<Hyphenated, _>(column)?.into_uuid())
}

pub(crate) fn parsed<T>(
    row: &SqliteRow,
    column: &str,
    parse: impl FnOnce(&str) -> corral_core::Result<T>,
) -> sqlx::Result<T> {
    parse(row.try_get(column)?).map_err(|err| sqlx::Error::ColumnDecode {
        index: column.to_owned(),
        source: Box::new(err),
    })
}
