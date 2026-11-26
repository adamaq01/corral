mod activity;
mod engine;
mod environment;
mod provider;
mod registry;
mod repos;
mod secrets;
mod settings;
mod world;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use activity::RecordedSignals;
pub use engine::{EngineCall, FakeEngine};
pub use environment::MemoryEnvironmentRepo;
pub use provider::FakeProvider;
pub use registry::FakeRegistry;
pub use repos::{MemoryKeyRepo, MemorySandboxRepo, MemoryUserRepo};
pub use secrets::MemorySecretStore;
pub use settings::MemorySettingsRepo;
pub use world::*;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
