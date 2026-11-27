use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use tracing::subscriber::{DefaultGuard, NoSubscriber};
use tracing::{Dispatch, Level};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;

use super::lock;

/// What Corral logs on this thread while it is held, debug and up, as the server prints it
/// without times or colors. Tasks spawned on other threads go uncaptured, so a test that needs
/// them runs on a current-thread runtime.
pub struct Logs {
    written: Written,
    _scope: DefaultGuard,
}

#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        lock(&self.0).extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// While a single dispatcher exists, tracing decides whether a callsite is wanted by asking only
// the subscriber of the thread that first reaches it, and keeps that answer for every thread. An
// idle second dispatcher makes it ask every subscriber, so a line that another test's thread
// reached first still reaches this one.
static IDLE: OnceLock<Dispatch> = OnceLock::new();

impl Logs {
    #[must_use]
    pub fn capture() -> Self {
        IDLE.get_or_init(|| Dispatch::new(NoSubscriber::default()));
        let written = Written::default();
        let writer = written.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::DEBUG)
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish()
            .with(Targets::new().with_target("corral", Level::DEBUG));
        Self {
            written,
            _scope: tracing::subscriber::set_default(subscriber),
        }
    }

    /// Every line so far.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(&lock(&self.written.0))
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The lines whose message is `message`.
    #[must_use]
    pub fn of(&self, message: &str) -> Vec<String> {
        let shown = format!(": {message}");
        self.lines()
            .into_iter()
            .filter(|line| {
                line.find(&shown).is_some_and(|at| {
                    let rest = &line[at + shown.len()..];
                    rest.is_empty() || rest.starts_with(' ')
                })
            })
            .collect()
    }

    /// The one line whose message is `message`.
    #[must_use]
    pub fn one(&self, message: &str) -> String {
        let found = self.of(message);
        assert_eq!(found.len(), 1, "one {message:?} in {:#?}", self.lines());
        found.into_iter().next().unwrap_or_default()
    }
}
