mod engine;
mod password_hasher;
mod repos;
mod secrets;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use engine::{EngineCall, FakeEngine};
pub use password_hasher::FakePasswordHasher;
pub use repos::{MemoryKeyRepo, MemoryLocalAccountRepo, MemorySandboxRepo, MemoryUserRepo};
pub use secrets::MemorySecretStore;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
