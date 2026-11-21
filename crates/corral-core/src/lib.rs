#[cfg(any(test, feature = "contract"))]
#[allow(clippy::missing_panics_doc)]
pub mod contract;
pub mod domain;
mod error;
#[cfg(any(test, feature = "fakes"))]
pub mod fakes;
pub mod ports;

pub use error::{Error, Result};
