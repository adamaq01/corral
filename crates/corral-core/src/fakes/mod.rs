mod engine;
mod repos;
mod secrets;
mod world;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use engine::{EngineCall, FakeEngine};
pub use repos::{MemoryKeyRepo, MemoryLocalAccountRepo, MemorySandboxRepo, MemoryUserRepo};
pub use secrets::MemorySecretStore;
pub use world::*;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
