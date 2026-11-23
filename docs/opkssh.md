# Logging in with opkssh

People who sign in to Corral with OIDC can reach their sandboxes over SSH without registering a
key. They run [opkssh](https://github.com/openpubkey/opkssh), which gets an ID token from the same
issuer as Corral's web login, through a public client of its own, and puts it in an SSH
certificate. Corral checks each certificate with the official `opkssh verify` (v0.20.0, in the
server image) and lets it in when the token's subject is the user being logged in as. Registered
keys work as before, and local accounts are not offered opkssh.

## Corral

```toml
[identity.oidc.opkssh]
client_id = "opkssh"
policy = "24h"
```

- `client_id` is the issuer's client for opkssh: the audience its tokens must have.
- `policy` is how long a certificate lasts from the token's issue time: `12h`, `24h` (the
  default), `48h` or `1week`. opkssh's `oidc` and `oidc_refreshed` policies are not offered: they
  follow the ID token's own expiry, which nothing renews ahead of.

With the Helm chart these are `identity.oidc.opkssh.enabled`, `clientId` and `policy`.

At startup Corral writes `/etc/opk/providers` (the issuer, the client id and the policy) and
`/etc/opk/auth_id` (one row admitting that audience under that issuer; Corral then checks the
subject itself). opkssh reads those paths only. It keeps the issuer's JWKS in `/var/cache/opkssh`
and logs to `/var/log/opkssh.log`. The server image makes all three for its user, uid 65532, and
Corral takes the cache over when it runs as root, as on Docker Compose. The chart mounts empty
directories there, since the root filesystem is read-only.

Each certificate login runs `opkssh verify` with a 5 second timeout. It runs with none of Corral's
environment but `SSL_CERT_FILE`, `SSL_CERT_DIR` and the proxy variables, so an issuer behind a
private CA works when `SSL_CERT_FILE` names that CA for Corral, for its web login too. A refused
certificate is logged with opkssh's reason; certificates are never logged. ContainerSSH waits 10
seconds for an answer about a key.

## VoidAuth

Give opkssh a client of its own, next to Corral's confidential `corral` client. Declared through
VoidAuth's environment (checked with VoidAuth 1.19.2):

```sh
OIDC_opkssh_CLIENT_AUTH_METHOD=none
OIDC_opkssh_CLIENT_REDIRECT_URLS=http://localhost:3000/login-callback, http://localhost:10001/login-callback, http://localhost:11110/login-callback
OIDC_opkssh_CLIENT_SKIP_CONSENT=true
```

or the same in VoidAuth's admin pages: auth method `none`, those three redirect URLs, and Skip
Consent. What opkssh needs from it:

- A public client: no secret, with PKCE (S256), which opkssh always sends.
- Exactly those redirect URIs: opkssh listens on the first free port of 3000, 10001 and 11110.
- The nonce in the ID token: opkssh's nonce commits to the user's key.
- RS256-signed ID tokens, with the keys at the discovery document's `jwks_uri`.
- An `https://` issuer, which is VoidAuth's `APP_URL` followed by `/oidc`.
- Skip Consent, together with `prompt: ""` in each user's opkssh config. opkssh asks for
  `prompt=consent` by default, and VoidAuth then always shows its consent page, whose form cannot
  send the browser back to `http://localhost` under VoidAuth's Content-Security-Policy
  (`form-action 'self' https:`), so the login never finishes.

Users have the same `sub`, VoidAuth's user id, at both clients, which is how a certificate maps to
the Corral user who signed in on the web. opkssh logins use no refresh tokens.

## What users do

Install opkssh: `brew install opkssh`, or a binary from its
[releases](https://github.com/openpubkey/opkssh/releases). A sandbox's page then shows, to
people signed in with single sign-on:

- an entry for `~/.opk/config.yml`, with the issuer, the client id and `prompt: ""`;
- a block for the end of `~/.ssh/config` whose `Match exec` hook runs
  `opkssh login corral -i ~/.ssh/corral-opkssh` before ssh connects, when that certificate is
  missing or written more than its lifetime less an hour ago, and gives up after 30 seconds.
  opkssh's certificates carry no expiry of their own, so the hook goes by the file's age.
  The block also sets `PreferredAuthentications publickey`: ContainerSSH offers password
  logins it always refuses, and without it ssh answers a refused certificate with a password
  prompt.

While the browser holds a VoidAuth session (sign in with Remember Me), the login opens a tab that
completes on its own in under a second. Without one, sign in within the 30 seconds, or run
`opkssh login corral -i ~/.ssh/corral-opkssh` first. Herdr reconnects through the same hook; when
it needs a login and no VoidAuth session is live, Herdr reports the machine unreachable until the
user signs in again.
