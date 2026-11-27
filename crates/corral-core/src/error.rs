use std::error::Error as StdError;

type Source = Box<dyn StdError + Send + Sync>;

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
    /// Forbidden by the account's own state, which the message tells its person.
    #[error("{0}")]
    Inactive(String),
    #[error("limit reached: {0}")]
    LimitReached(String),
    // Logs and the server print errors through Display, so it carries the whole cause chain.
    #[error("{context}: {}", chain(.source.as_ref()))]
    Internal {
        context: String,
        #[source]
        source: Source,
    },
}

impl Error {
    /// Whether it refuses what was asked, rather than failing at it.
    #[must_use]
    pub fn is_refusal(&self) -> bool {
        !matches!(self, Self::Internal { .. })
    }

    pub fn internal(context: impl Into<String>, source: impl Into<Source>) -> Self {
        Self::Internal {
            context: context.into(),
            source: source.into(),
        }
    }
}

fn chain(source: &(dyn StdError + 'static)) -> String {
    let mut text = source.to_string();
    let mut cause = source.source();
    while let Some(inner) = cause {
        text = format!("{text}: {inner}");
        cause = inner.source();
    }
    text
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::fmt;

    use super::Error;

    #[derive(Debug)]
    struct Wrapped(std::io::Error);

    impl fmt::Display for Wrapped {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("write failed")
        }
    }

    impl std::error::Error for Wrapped {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn internal_errors_keep_their_source_and_show_its_chain() {
        let error = Error::internal("store", Wrapped(std::io::Error::other("disk full")));
        assert_eq!(error.to_string(), "store: write failed: disk full");
        let source = error.source().expect("a source");
        assert!(source.downcast_ref::<Wrapped>().is_some());
    }
}
