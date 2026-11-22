pub mod app;
#[cfg(feature = "contract")]
#[allow(clippy::missing_panics_doc)]
pub mod contract;
pub mod domain;
mod error;
#[cfg(any(test, feature = "fakes"))]
pub mod fakes;
pub mod ports;
#[cfg(test)]
mod testing;

pub use error::{Error, Result};
