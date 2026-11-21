use crate::{ActivityError, Exec};

pub(crate) async fn matches(exec: &dyn Exec, argv: &[&str]) -> Result<bool, ActivityError> {
    match exec.run(argv).await?.code {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(ActivityError::UnexpectedExit {
            command: argv.join(" "),
            code,
        }),
    }
}
