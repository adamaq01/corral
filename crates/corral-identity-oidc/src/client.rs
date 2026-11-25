use std::time::Duration;

use async_trait::async_trait;
use corral_api::provider_path;
use corral_core::domain::{PictureUrl, Role};
use corral_core::ports::{Identity, IdentityProvider};
use corral_core::{Error, Result};
use openidconnect::core::{
    CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreErrorResponseType,
    CoreGenderClaim, CoreJsonWebKey, CoreJsonWebKeySet, CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm, CoreProviderMetadata, CoreRevocableToken, CoreRevocationErrorResponse,
    CoreTokenIntrospectionResponse, CoreTokenType,
};
use openidconnect::{
    AdditionalClaims, AuthorizationCode, ClaimsVerificationError, Client, ClientId, ClientSecret,
    CsrfToken, EmptyExtraTokenFields, EndpointMaybeSet, EndpointNotSet, EndpointSet,
    HttpClientError, IdTokenClaims, IdTokenFields, IssuerUrl, Nonce, PkceCodeChallenge,
    RedirectUrl, RequestTokenError, Scope, StandardErrorResponse, StandardTokenResponse,
    TokenResponse, TokenUrl, reqwest,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::OidcConfig;
use crate::login_secrets::LoginSecrets;
use crate::opkssh::{Opkssh, OpksshLogin};

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
    member_group: String,
    admin_group: String,
    http: reqwest::Client,
    opkssh: Option<Opkssh>,
}

impl OidcProvider {
    pub const ID: &str = "oidc";

    /// `base_url` is where the API is served; the issuer sends people back to its callback
    /// route there.
    pub async fn discover(
        config: OidcConfig,
        client_secret: &SecretString,
        base_url: &str,
    ) -> Result<Self> {
        let opkssh = match config.opkssh {
            Some(opkssh) => Some(Opkssh::install(&config.issuer, opkssh).await?),
            None => None,
        };
        let issuer =
            IssuerUrl::new(config.issuer).map_err(|err| Error::internal("issuer url", err))?;
        let callback = format!(
            "{}{}/callback",
            base_url.trim_end_matches('/'),
            provider_path(Self::ID)
        );
        let redirect_url =
            RedirectUrl::new(callback).map_err(|err| Error::internal("redirect url", err))?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|err| Error::internal("http client", err))?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|err| Error::internal("discovery", err))?;
        let token_url = metadata
            .token_endpoint()
            .cloned()
            .ok_or_else(|| Error::internal("discovery", "issuer has no token endpoint"))?;
        Ok(Self {
            metadata,
            token_url,
            client_id: ClientId::new(config.client_id),
            client_secret: ClientSecret::new(client_secret.expose_secret().to_owned()),
            redirect_url,
            scopes: config.scopes.into_iter().map(Scope::new).collect(),
            member_group: config.member_group,
            admin_group: config.admin_group,
            http,
            opkssh,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_opkssh(self, opkssh: Opkssh) -> Self {
        Self {
            opkssh: Some(opkssh),
            ..self
        }
    }

    /// How users log in with opkssh, when it is on.
    #[must_use]
    pub fn opkssh_login(&self) -> Option<&OpksshLogin> {
        self.opkssh.as_ref().map(Opkssh::login)
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
            .map_err(|err| Error::internal("jwks", err))
    }

    fn identity(&self, claims: &Claims) -> Result<Identity> {
        let username = claims
            .preferred_username()
            .ok_or_else(|| Error::internal("id token", "no preferred_username claim"))?;
        let groups = &claims.additional_claims().groups;
        let role = if groups.contains(&self.admin_group) {
            Some(Role::Admin)
        } else if groups.contains(&self.member_group) {
            Some(Role::Member)
        } else {
            None
        };
        Ok(Identity {
            subject: claims.subject().as_str().to_owned(),
            username: username.as_str().to_owned(),
            email: claims.email().map(|email| email.as_str().to_owned()),
            picture: claims
                .picture()
                .and_then(|picture| picture.get(None))
                .and_then(|picture| PictureUrl::parse(picture.as_str()).ok()),
            role,
        })
    }

    /// Where to send the person, and what the callback needs to check the answer.
    pub(crate) fn begin(&self, return_to: String) -> (String, LoginSecrets) {
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
        let secrets = LoginSecrets {
            csrf,
            nonce,
            pkce_verifier,
            return_to,
        };
        (url.into(), secrets)
    }

    /// `Forbidden` when the returned state is not the login's, before the code is redeemed.
    pub(crate) async fn finish(
        &self,
        secrets: LoginSecrets,
        state: Option<&str>,
        code: Option<&str>,
    ) -> Result<Identity> {
        if secrets.csrf != CsrfToken::new(state.unwrap_or_default().to_owned()) {
            return Err(Error::Forbidden);
        }
        let code = code
            .ok_or_else(|| Error::Invalid("the identity provider returned no code".to_owned()))?;
        let client = self.client(self.metadata.clone().set_jwks(self.current_keys().await?));
        let tokens = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(secrets.pkce_verifier)
            .request_async(&self.http)
            .await
            .map_err(token_error)?;
        let id_token = tokens
            .id_token()
            .ok_or_else(|| Error::internal("token endpoint", "response has no id token"))?;
        let claims = id_token
            .claims(&client.id_token_verifier(), &secrets.nonce)
            .map_err(claims_error)?;
        self.identity(claims)
    }
}

#[async_trait]
impl IdentityProvider for OidcProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    async fn vouches_for_ssh(&self, subject: &str, presented: &str) -> Result<bool> {
        let Some(opkssh) = &self.opkssh else {
            return Ok(false);
        };
        Ok(opkssh.subject(presented).await.as_deref() == Some(subject))
    }

    fn takes_ssh_certificates(&self) -> bool {
        self.opkssh.is_some()
    }
}

fn token_error(err: RequestTokenError<HttpError, TokenError>) -> Error {
    match err {
        RequestTokenError::ServerResponse(response)
            if *response.error() == CoreErrorResponseType::InvalidGrant =>
        {
            Error::Forbidden
        }
        RequestTokenError::ServerResponse(response) => {
            Error::internal("token endpoint", response.to_string())
        }
        RequestTokenError::Parse(..) => Error::internal("token endpoint", "unparsable response"),
        other => Error::internal("token request", other),
    }
}

fn claims_error(err: ClaimsVerificationError) -> Error {
    match err {
        ClaimsVerificationError::InvalidNonce(_) => Error::Forbidden,
        other => Error::internal("id token rejected", other),
    }
}
