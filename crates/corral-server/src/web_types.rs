use std::path::PathBuf;

use corral_api::TypeScript;
use corral_identity_local::LocalProvider;

const GENERATED: &str = "web/src/lib/api/generated/index.ts";

/// Every type the API and the login providers take and answer with.
fn render() -> String {
    let mut types = TypeScript::api();
    LocalProvider::typescript(&mut types);
    types.render()
}

#[test]
fn web_types_are_current() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(GENERATED);
    let generated = render();
    if std::env::var_os("CORRAL_WRITE_TYPES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
    }
    let checked_in = std::fs::read_to_string(&path).unwrap_or_default();
    let stale = checked_in
        .lines()
        .zip(generated.lines())
        .position(|(old, new)| old != new)
        .or_else(|| (checked_in != generated).then(|| checked_in.lines().count()));
    assert!(
        stale.is_none(),
        "{GENERATED} is stale from line {}: regenerate it with \
         `CORRAL_WRITE_TYPES=1 cargo test -p corral-server web_types`",
        stale.unwrap_or_default() + 1,
    );
}
