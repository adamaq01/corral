use corral_core::{Error, Result};

use crate::Exec;

pub(crate) async fn matches(exec: &dyn Exec, argv: &[&str]) -> Result<bool> {
    match exec.run(argv).await?.code {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(unexpected_exit(argv, code)),
    }
}

pub(crate) fn unexpected_exit(argv: &[&str], code: i32) -> Error {
    Error::internal(
        format!("`{}`", argv.join(" ")),
        format!("exited with code {code}"),
    )
}
