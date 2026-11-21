use std::collections::HashMap;

use async_trait::async_trait;
use corral_activity::{ActivityError, Exec, ExecOutput};

#[derive(Default)]
pub(crate) struct ScriptedExec {
    replies: HashMap<&'static str, Option<ExecOutput>>,
}

impl ScriptedExec {
    pub(crate) fn exits(mut self, command: &'static str, code: i32, stdout: &str) -> Self {
        let output = ExecOutput {
            code,
            stdout: stdout.to_owned(),
        };
        self.replies.insert(command, Some(output));
        self
    }

    pub(crate) fn fails(mut self, command: &'static str) -> Self {
        self.replies.insert(command, None);
        self
    }
}

#[async_trait]
impl Exec for ScriptedExec {
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput, ActivityError> {
        let command = argv.join(" ");
        self.replies
            .get(command.as_str())
            .unwrap_or_else(|| panic!("unscripted command: {command}"))
            .clone()
            .ok_or_else(|| ActivityError::Exec("pod exec failed".to_owned()))
    }
}
