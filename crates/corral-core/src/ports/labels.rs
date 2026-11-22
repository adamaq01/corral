use std::collections::BTreeMap;

use super::SandboxSpec;
use crate::domain::SandboxId;

pub const MANAGED_BY: &str = "app.kubernetes.io/managed-by";
pub const MANAGER: &str = "corral";
pub const SANDBOX_ID: &str = "corral.adamaq01.fr/sandbox-id";
const OWNER: &str = "corral.adamaq01.fr/owner";
const SANDBOX: &str = "corral.adamaq01.fr/sandbox";

#[must_use]
pub fn of(id: &SandboxId, spec: &SandboxSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MANAGED_BY.to_owned(), MANAGER.to_owned()),
        (OWNER.to_owned(), spec.owner.as_str().to_owned()),
        (SANDBOX.to_owned(), spec.name.as_str().to_owned()),
        (SANDBOX_ID.to_owned(), id.0.clone()),
    ])
}
