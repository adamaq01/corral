use std::error::Error as _;

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

pub(crate) fn openbao_error(action: &str, err: &ClientError) -> Error {
    let detail = match (err, status(err)) {
        (ClientError::APIError { errors, .. }, Some(code)) if !errors.is_empty() => {
            format!("status {code}: {}", errors.join("; "))
        }
        (_, Some(code)) => format!("status {code}"),
        (_, None) => {
            let mut detail = err.to_string();
            let mut cause = err.source();
            while let Some(inner) = cause {
                detail = format!("{detail}: {inner}");
                cause = inner.source();
            }
            detail
        }
    };
    Error::Secrets(format!("openbao {action}: {detail}"))
}
