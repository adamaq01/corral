use std::error::Error as StdError;
use std::time::Duration;

use async_trait::async_trait;
use corral_core::domain::Principal;
use corral_core::ports::{IdentityProvider, LoginStart, LoginState};
use corral_core::{Error, Result};
use openidconnect::core::{
    CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreErrorResponseType,
    CoreGenderClaim, CoreJsonWebKey, CoreJsonWebKeySet, CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm, CoreProviderMetadata, CoreRevocableToken, CoreRevocationErrorResponse,
    CoreTokenIntrospectionResponse, CoreTokenType,
};
use openidconnect::{
    AdditionalClaims, AuthorizationCode, ClaimsVerificationError, Client, ClientId, ClientSecret,
    CsrfToken, DiscoveryError, EmptyExtraTokenFields, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, HttpClientError, IdTokenClaims, IdTokenFields, IssuerUrl, Nonce,
    PkceCodeChallenge, RedirectUrl, RequestTokenError, Scope, StandardErrorResponse,
    StandardTokenResponse, TokenResponse, TokenUrl, reqwest,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::OidcConfig;
use crate::pending_login::PendingLogin;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize, Serialize)]
struct GroupsClaim {
    #[serde(default)]
    groups: Vec<String>,
}

impl AdditionalClaims for GroupsClaim {}

type Claims = IdTokenClaims<GroupsClaim, CoreGenderClaim>;

type Tokens = StandardTokenResponse<
    IdTokenFields<
        GroupsClaim,
        EmptyExtraTokenFields,
        CoreGenderClaim,
        CoreJweContentEncryptionAlgorithm,
        CoreJwsSigningAlgorithm,
    >,
    CoreTokenType,
>;

type TokenError = StandardErrorResponse<CoreErrorResponseType>;

type HttpError = HttpClientError<reqwest::Error>;

type OidcClient<HasTokenUrl = EndpointSet> = Client<
    GroupsClaim,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    TokenError,
    Tokens,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    CoreRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    HasTokenUrl,
    EndpointMaybeSet,
>;

pub struct OidcProvider {
    metadata: CoreProviderMetadata,
    token_url: TokenUrl,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect_url: RedirectUrl,
    scopes: Vec<Scope>,
    http: reqwest::Client,
}

impl OidcProvider {
    pub async fn discover(config: OidcConfig) -> Result<Self> {
        let issuer = IssuerUrl::new(config.issuer)
            .map_err(|err| Error::Identity(format!("issuer url: {err}")))?;
        let redirect_url = RedirectUrl::new(config.redirect_url)
            .map_err(|err| Error::Identity(format!("redirect url: {err}")))?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|err| Error::Identity(format!("http client: {}", chain(&err))))?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|err| discovery_error("discovery", &err))?;
        let token_url = metadata
            .token_endpoint()
            .cloned()
            .ok_or_else(|| Error::Identity("issuer has no token endpoint".to_owned()))?;
        Ok(Self {
            metadata,
            token_url,
            client_id: ClientId::new(config.client_id),
            client_secret: ClientSecret::new(config.client_secret.expose_secret().to_owned()),
            redirect_url,
            scopes: config.scopes.into_iter().map(Scope::new).collect(),
            http,
        })
    }

    fn client(&self, metadata: CoreProviderMetadata) -> OidcClient {
        OidcClient::<EndpointMaybeSet>::from_provider_metadata(
            metadata,
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_token_uri(self.token_url.clone())
        .set_redirect_uri(self.redirect_url.clone())
        .disable_openid_scope()
    }

    async fn current_keys(&self) -> Result<CoreJsonWebKeySet> {
        CoreJsonWebKeySet::fetch_async(self.metadata.jwks_uri(), &self.http)
            .await
            .map_err(|err| discovery_error("jwks", &err))
    }
}

#[async_trait]
impl IdentityProvider for OidcProvider {
    async fn begin(&self) -> Result<LoginStart> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, csrf, nonce) = self
            .client(self.metadata.clone())
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scopes(self.scopes.iter().cloned())
            .set_pkce_challenge(pkce_challenge)
            .url();
        let state = PendingLogin {
            csrf,
            nonce,
            pkce_verifier,
        }
        .encode()?;
        Ok(LoginStart {
            url: url.into(),
            state,
        })
    }

    async fn complete(
        &self,
        state: LoginState,
        code: &str,
        returned_state: &str,
    ) -> Result<Principal> {
        let pending = PendingLogin::decode(&state)?;
        if pending.csrf != CsrfToken::new(returned_state.to_owned()) {
            return Err(Error::Forbidden);
        }
        let client = self.client(self.metadata.clone().set_jwks(self.current_keys().await?));
        let tokens = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(pending.pkce_verifier)
            .request_async(&self.http)
            .await
            .map_err(token_error)?;
        let id_token = tokens
            .id_token()
            .ok_or_else(|| Error::Identity("token response has no id token".to_owned()))?;
        let claims = id_token
            .claims(&client.id_token_verifier(), &pending.nonce)
            .map_err(claims_error)?;
        principal(claims)
    }
}

fn principal(claims: &Claims) -> Result<Principal> {
    let preferred_username = claims
        .preferred_username()
        .ok_or_else(|| Error::Identity("id token has no preferred_username claim".to_owned()))?;
    Ok(Principal {
        subject: claims.subject().as_str().to_owned(),
        preferred_username: preferred_username.as_str().to_owned(),
        email: claims.email().map(|email| email.as_str().to_owned()),
        groups: claims.additional_claims().groups.clone(),
    })
}

fn token_error(err: RequestTokenError<HttpError, TokenError>) -> Error {
    match err {
        RequestTokenError::ServerResponse(response)
            if *response.error() == CoreErrorResponseType::InvalidGrant =>
        {
            Error::Forbidden
        }
        RequestTokenError::ServerResponse(response) => {
            Error::Identity(format!("token endpoint: {response}"))
        }
        RequestTokenError::Parse(..) => {
            Error::Identity("token endpoint sent an unparsable response".to_owned())
        }
        other => Error::Identity(format!("token request: {}", chain(&other))),
    }
}

fn claims_error(err: ClaimsVerificationError) -> Error {
    match err {
        ClaimsVerificationError::InvalidNonce(_) => Error::Forbidden,
        other => Error::Identity(format!("id token rejected: {}", chain(&other))),
    }
}

fn discovery_error(stage: &str, err: &DiscoveryError<HttpError>) -> Error {
    Error::Identity(format!("{stage}: {}", chain(err)))
}

fn chain(err: &dyn StdError) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}
