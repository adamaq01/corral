//! `corral-new`: makes the sandbox the login names from a JSON spec on stdin, waits until it
//! takes sessions, and prints what was made as JSON, for scripts.

use std::sync::Arc;

use russh::{ChannelId, ChannelMsg};
use tracing::warn;

use crate::api::{Create, Created, Made, Spec};
use crate::link::{Link, unreachable};
use crate::relay::FromClient;
use crate::session::REFUSED;

const COMMAND: &[u8] = b"corral-new";
const MAX_SPEC: usize = 64 * 1024;

pub(crate) fn is_script(command: &[u8]) -> bool {
    command.trim_ascii() == COMMAND
}

pub(crate) async fn run(link: Arc<Link>, id: ChannelId, mut from_client: FromClient) {
    let client = &link.client;
    let _ = client.channel_success(id).await;
    match made(&link, &mut from_client).await {
        Ok(made) => {
            let printed = serde_json::to_string(&made).expect("a spec serializes");
            let _ = client.data(id, format!("{printed}\n")).await;
            let _ = client.exit_status_request(id, 0).await;
        }
        Err(error) => {
            let printed = serde_json::json!({ "error": error });
            let _ = client.extended_data(id, 1, format!("{printed}\n")).await;
            let _ = client.exit_status_request(id, REFUSED).await;
        }
    }
    let _ = client.eof(id).await;
    let _ = client.close(id).await;
}

async fn made(link: &Link, from_client: &mut FromClient) -> Result<Made, String> {
    let spec = spec(from_client).await?;
    let create = Create {
        username: link.username.clone(),
        spec,
    };
    let made = match link.server().create(&create).await {
        Ok(Created::Made { made }) => made,
        Ok(Created::Refused { reason }) => return Err(reason),
        Err(error) => {
            warn!(%error, username = link.username, "could not ask Corral to make a sandbox");
            return Err(unreachable());
        }
    };
    link.wake().await?;
    Ok(made)
}

/// The spec the client sends before it closes its input; none at all is the empty spec.
async fn spec(from_client: &mut FromClient) -> Result<Spec, String> {
    let mut read = Vec::new();
    while let Some(mut taken) = from_client.next().await {
        match taken.message() {
            ChannelMsg::Data { data } => {
                if read.len() + data.len() > MAX_SPEC {
                    return Err(format!("the spec is over {} KiB", MAX_SPEC / 1024));
                }
                read.extend_from_slice(&data);
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    if read.trim_ascii().is_empty() {
        return Ok(Spec::default());
    }
    serde_json::from_slice(&read).map_err(|error| format!("the spec is not valid: {error}"))
}

#[cfg(test)]
mod tests {
    use super::is_script;
    use crate::api::Spec;

    #[test]
    fn only_corral_new_itself_is_the_script() {
        assert!(is_script(b"corral-new"));
        assert!(is_script(b" corral-new\n"));
        assert!(!is_script(b"corral-new --help"));
        assert!(!is_script(b"corral new"));
    }

    #[test]
    fn a_spec_takes_every_field_or_none_and_refuses_others() {
        let full: Spec = serde_json::from_str(
            r#"{"image": "dev", "size": "roomy", "environment": ["GH_TOKEN", ".gitconfig"]}"#,
        )
        .unwrap();
        assert_eq!(
            (full.image.as_deref(), full.size.as_deref()),
            (Some("dev"), Some("roomy"))
        );
        assert_eq!(full.environment.unwrap().len(), 2);
        assert_eq!(serde_json::from_str::<Spec>("{}").unwrap(), Spec::default());
        let error = serde_json::from_str::<Spec>(r#"{"sizes": "roomy"}"#).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `sizes`"),
            "{error}"
        );
    }
}
