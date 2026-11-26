mod activity;
mod engine;
mod provider;
mod registry;
mod repos;
mod secrets;
mod settings;
mod setup;
mod world;

use std::sync::{Mutex, MutexGuard, PoisonError};

pub use activity::RecordedSignals;
pub use engine::{EngineCall, FakeEngine};
pub use provider::FakeProvider;
pub use registry::FakeRegistry;
pub use repos::{MemoryKeyRepo, MemorySandboxRepo, MemoryUserRepo};
pub use secrets::MemorySecretStore;
pub use settings::MemorySettingsRepo;
pub use setup::{MemorySetupRepo, shared_file, shared_variable};
pub use world::*;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
