use corral_core::Error;

#[must_use]
pub fn db_error(err: sqlx::Error) -> Error {
    unique_as("already exists".to_owned())(err)
}

/// Maps a unique violation to a `Conflict` with `message`, which names what already exists:
/// the database's own message names tables and constraints, and conflicts reach API users.
pub fn unique_as(message: String) -> impl FnOnce(sqlx::Error) -> Error {
    move |err| match err {
        sqlx::Error::Database(db) if db.is_unique_violation() => Error::Conflict(message),
        err => Error::internal("store", err),
    }
}
