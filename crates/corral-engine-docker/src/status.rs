use bollard::models::{ContainerInspectResponse, ContainerStateStatusEnum as DockerState};
use corral_core::domain::{SandboxId, SandboxState, SandboxStatus};
use corral_core::{Error, Result};

pub(crate) fn is_running(container: &ContainerInspectResponse) -> bool {
    container.state.as_ref().and_then(|state| state.status) == Some(DockerState::RUNNING)
}

pub(crate) fn map(
    id: &SandboxId,
    container: Option<&ContainerInspectResponse>,
) -> Result<SandboxStatus> {
    let container = container.ok_or_else(|| Error::NotFound(format!("sandbox {}", id.0)))?;
    let state = container.state.clone().unwrap_or_default();
    let error = state.error.filter(|error| !error.is_empty());
    let (state, detail) = match state.status {
        Some(DockerState::REMOVING) => (SandboxState::Deleting, None),
        Some(DockerState::RUNNING) => (SandboxState::Running, None),
        Some(DockerState::RESTARTING | DockerState::EMPTY) | None => (SandboxState::Starting, None),
        Some(DockerState::DEAD) => (
            SandboxState::Failed,
            Some(error.unwrap_or_else(|| "the container is dead".to_owned())),
        ),
        Some(DockerState::EXITED | DockerState::CREATED | DockerState::PAUSED) => {
            if state.oom_killed == Some(true) {
                (SandboxState::Failed, Some("ran out of memory".to_owned()))
            } else if let Some(error) = error {
                (SandboxState::Failed, Some(error))
            } else {
                (SandboxState::Sleeping, None)
            }
        }
    };
    Ok(SandboxStatus { state, detail })
}

#[cfg(test)]
mod tests {
    use bollard::models::{ContainerInspectResponse, ContainerState, ContainerStateStatusEnum};
    use corral_core::Error;
    use corral_core::domain::{SandboxId, SandboxState};

    use super::map;

    fn container(
        status: ContainerStateStatusEnum,
        oom_killed: bool,
        error: &str,
    ) -> ContainerInspectResponse {
        ContainerInspectResponse {
            state: Some(ContainerState {
                status: Some(status),
                oom_killed: Some(oom_killed),
                error: Some(error.to_owned()),
                ..ContainerState::default()
            }),
            ..ContainerInspectResponse::default()
        }
    }

    #[test]
    fn status_mapping() {
        use ContainerStateStatusEnum::{CREATED, DEAD, EXITED, REMOVING, RESTARTING, RUNNING};
        let id = SandboxId("sbx-0123456789ab".to_owned());
        for (status, oom_killed, error, expected, detail) in [
            (RUNNING, false, "", SandboxState::Running, None),
            (RUNNING, true, "", SandboxState::Running, None),
            (EXITED, false, "", SandboxState::Sleeping, None),
            (CREATED, false, "", SandboxState::Sleeping, None),
            (
                EXITED,
                true,
                "",
                SandboxState::Failed,
                Some("ran out of memory"),
            ),
            (
                CREATED,
                false,
                "no such image",
                SandboxState::Failed,
                Some("no such image"),
            ),
            (
                DEAD,
                false,
                "",
                SandboxState::Failed,
                Some("the container is dead"),
            ),
            (RESTARTING, false, "", SandboxState::Starting, None),
            (REMOVING, false, "", SandboxState::Deleting, None),
        ] {
            let mapped = map(&id, Some(&container(status, oom_killed, error))).unwrap();
            assert_eq!(
                mapped.state, expected,
                "{status} oom={oom_killed} error={error:?}"
            );
            assert_eq!(mapped.detail.as_deref(), detail, "{status}");
        }
        assert!(matches!(map(&id, None), Err(Error::NotFound(_))));
    }
}
