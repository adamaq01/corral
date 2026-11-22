use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{ActivityError, Exec, ExecOutput};

#[derive(Clone)]
enum Reply {
    Exit(ExecOutput),
    Fail,
}

#[derive(Clone, Default)]
pub(crate) struct ScriptedExec {
    replies: HashMap<String, Reply>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl ScriptedExec {
    pub(crate) fn exits(mut self, command: &str, code: i32, stdout: &str) -> Self {
        let output = ExecOutput {
            code,
            stdout: stdout.to_owned(),
        };
        self.replies.insert(command.to_owned(), Reply::Exit(output));
        self
    }

    pub(crate) fn fails(mut self, command: &str) -> Self {
        self.replies.insert(command.to_owned(), Reply::Fail);
        self
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Exec for ScriptedExec {
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput, ActivityError> {
        let command = argv.join(" ");
        self.calls.lock().unwrap().push(command.clone());
        match self.replies.get(&command) {
            Some(Reply::Exit(output)) => Ok(output.clone()),
            Some(Reply::Fail) => Err(ActivityError::Exec("exec failed".to_owned())),
            None => panic!("unscripted command: {command}"),
        }
    }
}
