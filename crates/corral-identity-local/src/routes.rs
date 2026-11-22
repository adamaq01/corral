use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use corral_api::{
    ApiError, ClientAddress, CurrentAdmin, Identities, JsonBody, LoginKind, PathParam,
    ProviderDescription, ProviderRoutes, SignedIn, TypeScript, provider_path,
};
use corral_core::Error;
use corral_core::domain::{Handle, Role};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::LocalProvider;
use crate::password::MIN_PASSWORD_CHARS;

type Local = State<Arc<LocalProvider>>;

/// The same for an unknown username, so the answer never tells which accounts exist.
const WRONG_PASSWORD: &str = "wrong username or password";

#[derive(Deserialize, TS)]
struct PasswordLogin {
    username: String,
    #[ts(type = "string")]
    password: SecretString,
}

#[derive(Deserialize, TS)]
struct NewAccount {
    username: Handle,
    #[ts(type = "string")]
    password: SecretString,
    role: Role,
}

#[derive(Deserialize, TS)]
struct NewPassword {
    #[ts(type = "string")]
    password: SecretString,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "LocalAccount")]
struct AccountDto {
    username: Handle,
    role: Role,
    created_at: DateTime<Utc>,
    /// Whether the admin asking signs in with this account.
    yours: bool,
}

impl LocalProvider {
    /// The types its routes take and answer with, and the limits they enforce.
    pub fn typescript(types: &mut TypeScript) {
        types
            .add::<PasswordLogin>()
            .add::<NewAccount>()
            .add::<NewPassword>()
            .add::<AccountDto>()
            .limit(
                "MIN_PASSWORD_CHARS",
                "The fewest characters a local account's password may have.",
                MIN_PASSWORD_CHARS,
            );
    }

    /// `POST /login` with `{"username", "password"}`, and for admins the accounts under
    /// `/accounts`.
    #[must_use]
    pub fn routes(self: &Arc<Self>) -> ProviderRoutes {
        let path = provider_path(Self::ID);
        ProviderRoutes {
            id: Self::ID.to_owned(),
            description: ProviderDescription {
                label: "Local account".to_owned(),
                kind: LoginKind::Password,
                login: format!("{path}/login"),
                accounts: Some(format!("{path}/accounts")),
                opkssh: None,
            },
            routes: Router::new()
                .route("/login", post(login_route))
                .route("/accounts", get(list).post(create))
                .route("/accounts/{username}", delete(remove))
                .route("/accounts/{username}/password", put(set_password))
                .with_state(self.clone()),
        }
    }
}

async fn login_route(
    State(local): Local,
    ClientAddress(address): ClientAddress,
    identities: Identities,
    JsonBody(form): JsonBody<PasswordLogin>,
) -> Result<SignedIn, ApiError> {
    let attempt = local
        .limiter
        .admit(&form.username, address, Instant::now())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many failed logins, try again later",
            )
        })?;
    let verified = local.verify(&form.username, &form.password).await;
    if !matches!(verified, Err(Error::Forbidden)) {
        attempt.release();
    }
    let identity = verified.map_err(|error| match error {
        Error::Forbidden => ApiError::new(StatusCode::FORBIDDEN, WRONG_PASSWORD),
        other => other.into(),
    })?;
    identities.sign_in(identity).await
}

async fn list(
    _: CurrentAdmin,
    State(local): Local,
    identities: Identities,
) -> Result<Json<Vec<AccountDto>>, ApiError> {
    let accounts = local.accounts.list().await?;
    let yours = identities.signed_in_subject().await?;
    Ok(Json(
        accounts
            .into_iter()
            .map(|account| AccountDto {
                yours: yours.as_deref() == Some(account.username.as_str()),
                username: account.username,
                role: account.role,
                created_at: account.created_at,
            })
            .collect(),
    ))
}

async fn create(
    _: CurrentAdmin,
    State(local): Local,
    JsonBody(account): JsonBody<NewAccount>,
) -> Result<StatusCode, ApiError> {
    local
        .create(&account.username, account.password, account.role)
        .await?;
    Ok(StatusCode::CREATED)
}

async fn set_password(
    _: CurrentAdmin,
    State(local): Local,
    PathParam(username): PathParam<String>,
    JsonBody(body): JsonBody<NewPassword>,
) -> Result<StatusCode, ApiError> {
    local.set_password(&username, body.password).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the account with the user it logged in as, if it ever did.
async fn remove(
    CurrentAdmin(admin): CurrentAdmin,
    State(local): Local,
    identities: Identities,
    PathParam(username): PathParam<String>,
) -> Result<StatusCode, ApiError> {
    let existed = local.exists(&username).await?;
    if identities.delete(&admin, &username).await? || existed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(Error::NotFound(format!("local account {username}")).into())
    }
}
