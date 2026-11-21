use std::time::Duration;

use async_trait::async_trait;
use corral_activity::{ActivityError, Exec, ExecOutput};
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use tokio::time::timeout;

use crate::exec;

const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct PodExec<'a> {
    pods: &'a Api<Pod>,
    pod: &'a str,
}

impl<'a> PodExec<'a> {
    pub(crate) fn new(pods: &'a Api<Pod>, pod: &'a str) -> Self {
        Self { pods, pod }
    }
}

#[async_trait]
impl Exec for PodExec<'_> {
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput, ActivityError> {
        // Exec of a missing binary is an API error; through sh it is exit 127, which probes read.
        let argv: Vec<&str> = ["sh", "-c", "exec \"$@\"", "sh"]
            .iter()
            .chain(argv)
            .copied()
            .collect();
        let (code, stdout) = timeout(TIMEOUT, exec::run(self.pods, self.pod, &argv))
            .await
            .map_err(|_| {
                ActivityError::Exec(format!("no answer from {} in {TIMEOUT:?}", self.pod))
            })?
            .map_err(|error| ActivityError::Exec(error.to_string()))?;
        Ok(ExecOutput { code, stdout })
    }
}
