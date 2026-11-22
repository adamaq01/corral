use corral_core::Error;

pub(crate) fn kube_error(error: kube::Error) -> Error {
    match error {
        kube::Error::Api(status) if status.is_not_found() => Error::NotFound(status.message),
        error => Error::internal("kubernetes", error),
    }
}
