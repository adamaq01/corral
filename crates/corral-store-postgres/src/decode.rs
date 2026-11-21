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
