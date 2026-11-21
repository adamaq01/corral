use bollard::errors::Error as DockerError;
use corral_core::Error;

pub(crate) fn is_not_found(err: &DockerError) -> bool {
    status(err) == Some(404)
}

pub(crate) fn engine(action: &str, err: &DockerError) -> Error {
    Error::Engine(format!("{action}: {err}"))
}

fn status(err: &DockerError) -> Option<u16> {
    match err {
        DockerError::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    }
}
