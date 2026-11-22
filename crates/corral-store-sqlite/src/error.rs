use corral_core::Error;

pub(crate) fn db_error(err: sqlx::Error) -> Error {
    match err {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            Error::Conflict(db.message().to_owned())
        }
        err => Error::internal("store", err),
    }
}
