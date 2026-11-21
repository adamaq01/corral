mod engine;
mod repos;
mod secrets;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use engine::{EngineCall, FakeEngine};
pub use repos::{MemoryCredentialRepo, MemoryKeyRepo, MemorySandboxRepo, MemoryUserRepo};
pub use secrets::{MemorySecretBlobRepo, MemorySecretStore};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
