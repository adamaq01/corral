use std::collections::BTreeMap;

use super::SandboxSpec;
use crate::domain::SandboxId;

pub const MANAGED_BY: &str = "app.kubernetes.io/managed-by";
pub const MANAGER: &str = "corral";
pub const SANDBOX_ID: &str = "corral.adamaq01.fr/sandbox-id";
const OWNER: &str = "corral.adamaq01.fr/owner";
const USER_ID: &str = "corral.adamaq01.fr/user-id";
const SANDBOX: &str = "corral.adamaq01.fr/sandbox";

/// What every object an engine makes for a sandbox carries, so an operator can tell whose it is:
/// the owner's handle and user id, and the sandbox's name and id.
#[must_use]
pub fn of(id: &SandboxId, spec: &SandboxSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MANAGED_BY.to_owned(), MANAGER.to_owned()),
        (OWNER.to_owned(), label_value(spec.owner.as_str())),
        (USER_ID.to_owned(), spec.user.0.to_string()),
        (SANDBOX.to_owned(), label_value(spec.name.as_str())),
        (SANDBOX_ID.to_owned(), id.0.clone()),
    ])
}

/// The owner and the sandbox as written, for an engine whose objects also take annotations.
#[must_use]
pub fn annotations(spec: &SandboxSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (OWNER.to_owned(), spec.owner.as_str().to_owned()),
        (USER_ID.to_owned(), spec.user.0.to_string()),
        (SANDBOX.to_owned(), spec.name.as_str().to_owned()),
    ])
}

// A Kubernetes label value starts and ends with a letter or digit, which a handle or a sandbox
// name need not.
fn label_value(slug: &str) -> String {
    slug.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use uuid::Uuid;

    use super::{annotations, of};
    use crate::domain::{Allocation, Handle, Resources, SandboxId, SandboxName, UserId};
    use crate::ports::SandboxSpec;

    fn spec(handle: &str, name: &str) -> SandboxSpec {
        SandboxSpec {
            owner: Handle::parse(handle).unwrap(),
            user: UserId(Uuid::nil()),
            name: SandboxName::parse(name).unwrap(),
            image: "ubuntu:24.04".to_owned(),
            resources: Resources {
                cpu_millis: Allocation::new(500, 2000).unwrap(),
                memory_bytes: Allocation::new(1 << 30, 4 << 30).unwrap(),
                home_bytes: NonZeroU64::new(10 << 30).unwrap(),
            },
        }
    }

    #[test]
    fn objects_name_their_owner_and_sandbox() {
        let id = SandboxId("adam-main-0a1b2c".to_owned());
        let labels = of(&id, &spec("-adam-", "main-"));
        let pairs: Vec<(&str, &str)> = labels
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("app.kubernetes.io/managed-by", "corral"),
                ("corral.adamaq01.fr/owner", "adam"),
                ("corral.adamaq01.fr/sandbox", "main"),
                ("corral.adamaq01.fr/sandbox-id", "adam-main-0a1b2c"),
                (
                    "corral.adamaq01.fr/user-id",
                    "00000000-0000-0000-0000-000000000000"
                ),
            ]
        );
        assert_eq!(
            annotations(&spec("-adam-", "main-"))["corral.adamaq01.fr/owner"],
            "-adam-"
        );
    }
}
