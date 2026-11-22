use corral_core::Error;
use vaultrs::error::ClientError;

pub(crate) fn status(err: &ClientError) -> Option<u16> {
    match err {
        ClientError::APIError { code, .. }
        | ClientError::RestClientError {
            source: rustify::errors::ClientError::ServerResponseError { code, .. },
        } => Some(*code),
        _ => None,
    }
}

pub(crate) fn openbao_error(action: &str, err: ClientError) -> Error {
    let context = format!("openbao {action}");
    match (&err, status(&err)) {
        (ClientError::APIError { errors, .. }, Some(code)) if !errors.is_empty() => {
            Error::internal(context, format!("status {code}: {}", errors.join("; ")))
        }
        (_, Some(code)) => Error::internal(context, format!("status {code}")),
        (_, None) => Error::internal(context, err),
    }
}
