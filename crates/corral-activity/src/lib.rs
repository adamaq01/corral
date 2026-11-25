//! When a running sandbox sleeps: after its idle timeout with no work, from the signals that it
//! is at work, never from probes run inside it. Two sources feed it: the front door, which sees
//! a person or a client work through it, and the sandbox, which reports what it uses.

mod decide;
mod lock;
mod signals;
mod sweep;

pub use lock::{ActivityLock, Waking};
pub use signals::{Busy, Signals};
pub use sweep::{Settings, Sweepable, spawn};
