use bollard::errors::Error as DockerError;

pub(crate) fn is_not_found(err: &DockerError) -> bool {
    status(err) == Some(404)
}

fn status(err: &DockerError) -> Option<u16> {
    match err {
        DockerError::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    }
}
