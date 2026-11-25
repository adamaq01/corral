use std::io::Write;
use std::process::{Command, Output};

use tempfile::NamedTempFile;

fn check(config: &str) -> Output {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(config.as_bytes()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_corral-server"))
        .arg("--config")
        .arg(file.path())
        .args(["config", "check"])
        .env_clear()
        .output()
        .unwrap()
}

#[test]
fn a_valid_config_passes_without_any_environment() {
    let output = check(
        "[http]\nbase_url = \"http://corral.lan\"\n[identity.local]\n[ssh]\npublic_host = \"corral.lan\"\n\
         [engine]\nkind = \"agent-sandbox\"\nnamespace = \"n\"\nservice_account = \"s\"",
    );
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("is valid"));
}

#[test]
fn an_invalid_config_fails_with_the_reason() {
    let output =
        check("[http]\nbase_url = \"http://corral.lan\"\n[ssh]\npublic_host = \"corral.lan\"");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no login method"), "{stderr}");
}
