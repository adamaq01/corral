use async_trait::async_trait;

use crate::{ActivityError, Exec};

#[async_trait]
pub trait ActivityProbe: Send + Sync {
    async fn is_active(&self, exec: &dyn Exec) -> Result<bool, ActivityError>;
}

pub async fn any_active(
    probes: &[Box<dyn ActivityProbe>],
    exec: &dyn Exec,
) -> Result<bool, ActivityError> {
    for probe in probes {
        if probe.is_active(exec).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SshSessionProbe;
    use crate::testing::ScriptedExec;

    fn probes(patterns: &[&str]) -> Vec<Box<dyn ActivityProbe>> {
        patterns
            .iter()
            .map(|pattern| Box::new(SshSessionProbe::new(pattern)) as Box<dyn ActivityProbe>)
            .collect()
    }

    #[tokio::test]
    async fn active_when_any_probe_is_active() {
        let exec = ScriptedExec::default()
            .exits("pgrep -f first", 1, "")
            .exits("pgrep -f second", 0, "7\n");

        let active = any_active(&probes(&["first", "second"]), &exec).await;

        assert!(active.unwrap());
    }

    #[tokio::test]
    async fn inactive_when_every_probe_is_inactive() {
        let exec = ScriptedExec::default()
            .exits("pgrep -f first", 1, "")
            .exits("pgrep -f second", 1, "");

        let active = any_active(&probes(&["first", "second"]), &exec).await;

        assert!(!active.unwrap());
    }

    #[tokio::test]
    async fn probe_error_propagates() {
        let exec = ScriptedExec::default()
            .exits("pgrep -f first", 1, "")
            .fails("pgrep -f second");

        let active = any_active(&probes(&["first", "second"]), &exec).await;

        assert!(matches!(active, Err(ActivityError::Exec(_))));
    }
}
