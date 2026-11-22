use bollard::models::{ContainerInspectResponse, ContainerStateStatusEnum as DockerState};
use corral_core::domain::{SandboxId, SandboxStatus};
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
    Ok(match state.status {
        Some(DockerState::REMOVING) => SandboxStatus::Deleting,
        Some(DockerState::RUNNING) => SandboxStatus::Running,
        Some(DockerState::RESTARTING | DockerState::EMPTY) | None => SandboxStatus::Starting,
        Some(DockerState::DEAD) => {
            SandboxStatus::Failed(error.unwrap_or_else(|| "the container is dead".to_owned()))
        }
        Some(DockerState::EXITED | DockerState::CREATED | DockerState::PAUSED) => {
            if state.oom_killed == Some(true) {
                SandboxStatus::Failed("ran out of memory".to_owned())
            } else if let Some(error) = error {
                SandboxStatus::Failed(error)
            } else {
                SandboxStatus::Sleeping
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use bollard::models::{ContainerInspectResponse, ContainerState, ContainerStateStatusEnum};
    use corral_core::Error;
    use corral_core::domain::{SandboxId, SandboxStatus};

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
        let failed = |detail: &str| SandboxStatus::Failed(detail.to_owned());
        for (status, oom_killed, error, expected) in [
            (RUNNING, false, "", SandboxStatus::Running),
            (RUNNING, true, "", SandboxStatus::Running),
            (EXITED, false, "", SandboxStatus::Sleeping),
            (CREATED, false, "", SandboxStatus::Sleeping),
            (EXITED, true, "", failed("ran out of memory")),
            (CREATED, false, "no such image", failed("no such image")),
            (DEAD, false, "", failed("the container is dead")),
            (RESTARTING, false, "", SandboxStatus::Starting),
            (REMOVING, false, "", SandboxStatus::Deleting),
        ] {
            assert_eq!(
                map(&id, Some(&container(status, oom_killed, error))).unwrap(),
                expected,
                "{status} oom={oom_killed} error={error:?}"
            );
        }
        assert!(matches!(map(&id, None), Err(Error::NotFound(_))));
    }
}
