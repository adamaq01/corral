mod error;
mod migrations;
mod pool;
mod stored;
#[cfg(feature = "testing")]
#[allow(clippy::missing_panics_doc)]
pub mod testing;

pub use crate::error::{db_error, unique_as};
pub use crate::migrations::Migrations;
pub use crate::pool::Pool;
pub use crate::stored::{parsed, variant, variant_name};

// Expands the body once per driver, so each query compiles against that driver's native types.
// SQLite binds `$N` placeholders by number, so one SQL text serves both.
#[macro_export]
macro_rules! on_pool {
    ($pool:expr, |$conn:ident| $body:expr) => {
        match &$pool {
            $crate::Pool::Sqlite($conn) => $body,
            $crate::Pool::Postgres($conn) => $body,
        }
    };
}
