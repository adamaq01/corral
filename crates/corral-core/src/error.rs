#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("forbidden")]
    Forbidden,
    #[error("limit reached: {0}")]
    LimitReached(String),
    #[error("engine: {0}")]
    Engine(String),
    #[error("store: {0}")]
    Store(String),
    #[error("secrets: {0}")]
    Secrets(String),
    #[error("identity: {0}")]
    Identity(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
