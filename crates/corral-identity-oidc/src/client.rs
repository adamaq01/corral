use std::time::Duration;

use async_trait::async_trait;
use corral_core::domain::Role;
use corral_core::ports::{Answer, Challenge, Identity, IdentityProvider, Method};
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
}

impl OidcProvider {
    pub const ID: &str = "oidc";

    pub async fn discover(
        config: OidcConfig,
        client_secret: &SecretString,
        redirect_url: String,
    ) -> Result<Self> {
        let issuer =
            IssuerUrl::new(config.issuer).map_err(|err| Error::internal("issuer url", err))?;
        let redirect_url =
            RedirectUrl::new(redirect_url).map_err(|err| Error::internal("redirect url", err))?;
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
            role,
        })
    }
}

#[async_trait]
impl IdentityProvider for OidcProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn method(&self) -> Method {
        Method::Redirect
    }

    async fn challenge(&self) -> Result<Challenge> {
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
        let pending = LoginSecrets {
            csrf,
            nonce,
            pkce_verifier,
        }
        .encode()?;
        Ok(Challenge::Redirect {
            url: url.into(),
            pending,
        })
    }

    async fn verify(&self, answer: Answer) -> Result<Identity> {
        let Answer::Redirect { pending, params } = answer else {
            return Err(Error::Invalid("OIDC logs in through a redirect".to_owned()));
        };
        let secrets = LoginSecrets::decode(&pending)?;
        let returned_state = params.get("state").map_or("", String::as_str);
        if secrets.csrf != CsrfToken::new(returned_state.to_owned()) {
            return Err(Error::Forbidden);
        }
        let code = params
            .get("code")
            .ok_or_else(|| Error::Invalid("the identity provider returned no code".to_owned()))?;
        let client = self.client(self.metadata.clone().set_jwks(self.current_keys().await?));
        let tokens = client
            .exchange_code(AuthorizationCode::new(code.clone()))
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
