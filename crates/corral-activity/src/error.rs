#[derive(Debug, thiserror::Error)]
pub enum ActivityError {
    #[error("exec failed: {0}")]
    Exec(String),
    #[error("`{command}` exited with code {code}")]
    UnexpectedExit { command: String, code: i32 },
    #[error("unreadable `hc ls --json` output: {0}")]
    Parse(String),
}
