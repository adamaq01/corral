#!/usr/bin/env bash
# opkssh logins end to end: VoidAuth (upstream, behind TLS) as the issuer, Corral's image and
# ContainerSSH from deploy/compose, the real opkssh client, and Playwright for every browser step.
# CORRAL_SERVER_IMAGE and CORRAL_SANDBOX_IMAGE name prebuilt images, as for compose.sh. Needs
# web/node_modules (npm ci) and Playwright's Chromium (npx playwright install chromium).
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
helpers=$repo/tests/e2e/opkssh
project=corral-opkssh-e2e
sandbox_network=corral-opkssh-e2e-sandboxes
voidauth_image=voidauth/voidauth:1.19.2
voidauth=$project-voidauth
http_port=18180
ssh_port=48392
voidauth_port=13380
idp_port=19443
mint_port=19444
base=http://127.0.0.1:$http_port

started=$SECONDS
passed=0
work=
pids=()

ok() {
  passed=$((passed + 1))
  echo "ok $passed - $*"
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

expect() {
  if [[ "$3" != "$2" ]]; then
    fail "$1: expected [$2], got [$3]"
  fi
}

remove_stack() {
  local container volumes volume
  for container in $(docker ps --all --quiet --filter "network=$sandbox_network"); do
    volumes=$(docker inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' "$container")
    docker rm --force "$container" >/dev/null
    for volume in $volumes; do
      docker volume rm "$volume" >/dev/null
    done
  done
  (cd / && env -u COMPOSE_FILE -u COMPOSE_PROJECT_NAME \
    docker compose --project-name "$project" down --volumes --remove-orphans --timeout 5 >/dev/null 2>&1)
  docker rm --force --volumes "$voidauth" >/dev/null 2>&1 || true
  docker network rm "$sandbox_network" >/dev/null 2>&1 || true
}

cleanup() {
  local status=$?
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  if ((status != 0)) && [[ -n "$work" ]]; then
    tail -n 40 "$work/run.log" >&2 2>/dev/null || true
    (cd "$work" && docker compose logs --no-color --tail 40 >&2 2>&1) || true
    docker logs --tail 20 "$voidauth" >&2 2>&1 || true
  fi
  remove_stack || true
  if [[ -n "$work" ]]; then
    rm -rf "$work"
  fi
  return "$status"
}
trap cleanup EXIT
trap 'echo "FAIL: line $LINENO: $BASH_COMMAND" >&2' ERR

for tool in docker ssh ssh-keygen curl jq node openssl; do
  command -v "$tool" >/dev/null || fail "$tool is not on PATH"
done
[[ -d "$repo/web/node_modules/playwright" ]] || fail "no Playwright in web/node_modules: run npm ci in web"
remove_stack
work=$(mktemp -d "${TMPDIR:-/tmp}/corral-opkssh-e2e.XXXXXX")
log=$work/run.log
# The issuer must be one address for the host, which runs opkssh and the browser, and for Corral's
# container: the default bridge's gateway is the host on both sides.
gateway=$(docker network inspect bridge --format '{{(index .IPAM.Config 0).Gateway}}')
issuer=https://$gateway:$idp_port/oidc
export NO_PROXY="${NO_PROXY:+$NO_PROXY,}$gateway" no_proxy="${no_proxy:+$no_proxy,}$gateway"
export PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-$HOME/.cache/ms-playwright}

server_image=${CORRAL_SERVER_IMAGE:-corral-server:e2e}
sandbox_image=${CORRAL_SANDBOX_IMAGE:-corral-sandbox:dev}
if [[ -z "${CORRAL_SERVER_IMAGE:-}" ]]; then
  docker build --file "$repo/images/server/Dockerfile" --tag "$server_image" "$repo" >>"$log" 2>&1
fi
if [[ -z "${CORRAL_SANDBOX_IMAGE:-}" ]]; then
  docker build --tag "$sandbox_image" "$repo/images/sandbox" >>"$log" 2>&1
fi
docker pull --quiet "$voidauth_image" >>"$log"
ok "images: server $server_image, sandbox $sandbox_image, issuer $voidauth_image"

mkdir -m 700 "$work/bin" "$work/home" "$work/home/.ssh" "$work/home/.opk" "$work/mint"
version=$(sed -n 's/^ARG OPKSSH_VERSION=//p' "$repo/images/server/Dockerfile")
case "$(uname -m)" in
  x86_64 | amd64) arch=amd64 ;;
  aarch64 | arm64) arch=arm64 ;;
  *) fail "no opkssh build for $(uname -m)" ;;
esac
sha256=$(sed -n "s/^ARG OPKSSH_SHA256_${arch^^}=//p" "$repo/images/server/Dockerfile")
curl -fsSL -o "$work/bin/opkssh" \
  "https://github.com/openpubkey/opkssh/releases/download/v$version/opkssh-linux-$arch"
echo "$sha256  $work/bin/opkssh" | sha256sum -c - >>"$log"
chmod 755 "$work/bin/opkssh"
ok "the opkssh $version client, the version and checksum the server image pins"

openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 -subj /CN=corral-e2e-ca \
  -keyout "$work/ca.key" -out "$work/ca.pem" 2>>"$log"
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=issuer \
  -keyout "$work/tls.key" -out "$work/tls.csr" 2>>"$log"
printf 'subjectAltName=IP:%s\n' "$gateway" >"$work/san.ext"
openssl x509 -req -in "$work/tls.csr" -CA "$work/ca.pem" -CAkey "$work/ca.key" -CAcreateserial \
  -days 1 -extfile "$work/san.ext" -out "$work/tls.pem" 2>>"$log"
chmod 644 "$work/ca.pem"
export SSL_CERT_FILE=$work/ca.pem

corral_secret=$(openssl rand -hex 24)
redirects="http://localhost:3000/login-callback, http://localhost:10001/login-callback, http://localhost:11110/login-callback"
docker run --detach --name "$voidauth" --publish "127.0.0.1:$voidauth_port:3000" \
  --volume /app/config --volume /app/db \
  --env "APP_URL=https://$gateway:$idp_port" --env "STORAGE_KEY=$(openssl rand -hex 24)" \
  --env DB_ADAPTER=sqlite --env SESSION_DOMAIN=null \
  --env "OIDC_corral_CLIENT_SECRET=$corral_secret" \
  --env "OIDC_corral_CLIENT_REDIRECT_URLS=$base/auth/oidc/callback" \
  --env OIDC_corral_CLIENT_SKIP_CONSENT=true \
  --env OIDC_opkssh_CLIENT_AUTH_METHOD=none \
  --env "OIDC_opkssh_CLIENT_REDIRECT_URLS=$redirects" \
  --env OIDC_opkssh_CLIENT_SKIP_CONSENT=true \
  --env OIDC_other_CLIENT_AUTH_METHOD=none \
  --env "OIDC_other_CLIENT_REDIRECT_URLS=$redirects" \
  --env OIDC_other_CLIENT_SKIP_CONSENT=true \
  "$voidauth_image" >>"$log"
node "$helpers/tls-proxy.mjs" "$gateway" "$idp_port" "$voidauth_port" "$work/tls.pem" "$work/tls.key" \
  >>"$log" 2>&1 &
pids+=($!)
for _ in $(seq 120); do
  curl -sf -o /dev/null "$issuer/.well-known/openid-configuration" && break
  sleep 0.5
done
expect "VoidAuth's issuer" "$issuer" "$(curl -sS "$issuer/.well-known/openid-configuration" | jq -r .issuer)"
reset_link=$(docker logs "$voidauth" 2>&1 | grep -o "https://$gateway:$idp_port/reset_password?[^ ]*" | head -n 1)
admin_password="E2e-$(openssl rand -hex 12)"
node "$helpers/browser.mjs" reset "$reset_link" "$admin_password" >>"$log" 2>&1
ok "VoidAuth at $issuer with clients corral (confidential), opkssh and other (public, PKCE)"

cp -R "$repo/deploy/compose/." "$work/"
cd "$work"
sed -e "s|^base_url = .*|base_url = \"$base\"|" -e 's|^public_host = .*|public_host = "127.0.0.1"|' \
  "$repo/deploy/compose/config.toml" >config.toml
cat >>config.toml <<EOF
public_port = $ssh_port

[identity.oidc]
issuer = "$issuer"
client_id = "corral"
member_group = "auth_admins"

[identity.oidc.opkssh]
client_id = "opkssh"

[engine]
kind = "docker"
network = "$sandbox_network"

[[catalog.images]]
key = "dev"
label = "Local build"
reference = "$sandbox_image"
EOF
cat >compose.e2e.yaml <<EOF
services:
  corral:
    image: $server_image
    environment:
      SSL_CERT_FILE: /etc/corral/ca.pem
    volumes:
      - ./ca.pem:/etc/corral/ca.pem:ro
    ports: !override
      - "127.0.0.1:$http_port:8080"
  containerssh:
    ports: !override
      - "127.0.0.1:$ssh_port:2222"
networks:
  corral-sandboxes:
    name: $sandbox_network
EOF
export COMPOSE_PROJECT_NAME=$project COMPOSE_FILE=compose.yaml:compose.e2e.yaml
docker compose run --rm corral keys generate >.env 2>>"$log"
for key in containerssh_host_key containerssh_proxy_key; do
  ssh-keygen -q -t ed25519 -N '' -C containerssh -f "$key" && chmod 644 "$key"
done
{
  echo "CORRAL_PROXY_PUBLIC_KEY=$(cat containerssh_proxy_key.pub)"
  echo "CORRAL_OIDC_CLIENT_SECRET=$corral_secret"
} >>.env
docker compose up -d --wait >>"$log" 2>&1
expect "Corral's auth_id" "corral oidc:\"aud\":\"opkssh\" $issuer" \
  "$(docker compose cp corral:/etc/opk/auth_id - | tar -xO)"
expect "Corral's providers" "$issuer opkssh 24h" "$(docker compose cp corral:/etc/opk/providers - | tar -xO)"
ok "corral (as root, as Compose runs it) wrote opkssh's providers and its one auth_id row"

root_password=$(openssl rand -base64 18)
docker compose exec -T corral corral-server local-account create root --admin <<<"$root_password" >>"$log"
jq -cn --arg p "$root_password" '{username: "root", password: $p}' \
  | curl -sS --fail-with-body -c "$work/root.jar" -H 'Content-Type: application/json' \
    --data @- "$base/auth/local/login" >/dev/null
api() {
  local args=(-sS --fail-with-body -X "$1" -b "$work/root.jar" -H 'Content-Type: application/json')
  if (($# > 2)); then
    args+=(--data "$3")
  fi
  curl "${args[@]}" "$base/$2"
}
eve_password=$(openssl rand -base64 18)
api POST auth/local/accounts \
  "$(jq -cn --arg p "$eve_password" '{username: "eve", password: $p, role: "member"}')" >/dev/null
node "$helpers/browser.mjs" no-card "$base" eve "$eve_password" >>"$log" 2>&1
ok "local accounts root (admin) and eve; eve's sandbox page offers no opkssh"

profile=$work/profile
node "$helpers/browser.mjs" sign-in "$base" "$profile" auth_admin "$admin_password" "$work" >>"$log" 2>&1
expect "the signed-in OIDC user" "auth-admin" \
  "$(api GET api/admin/users | jq -r '.[] | select(.handle == "auth-admin") | .handle')"
grep -qx "    issuer: $issuer" opk-config.yml || fail "the page's opkssh config: $(cat opk-config.yml)"
grep -qx '    prompt: ""' opk-config.yml || fail "the page's opkssh config sets no empty prompt"
grep -q '^Match host 127.0.0.1 exec "sh -c ' ssh-hook.conf || fail "the page's hook: $(cat ssh-hook.conf)"
ok "auth_admin signed in to Corral through VoidAuth, created main, and copied the page's snippets"

cp opk-config.yml "$work/home/.opk/config.yml"
# The page's IdentityFile names ~/.ssh, which ssh reads from the passwd entry, not from HOME.
sed "s|^  IdentityFile ~/|  IdentityFile $work/home/|" ssh-hook.conf >ssh-hook.local
echo "[127.0.0.1]:$ssh_port $(cut -d' ' -f1,2 containerssh_host_key.pub)" >known_hosts
{
  cat <<EOF
Host corral-e2e
  HostName 127.0.0.1
  Port $ssh_port
  IdentitiesOnly yes
  IdentityAgent none
  BatchMode yes
  PreferredAuthentications publickey
  UserKnownHostsFile $work/known_hosts
  StrictHostKeyChecking yes
  ConnectTimeout 10
  LogLevel ERROR
EOF
  cat ssh-hook.local
} >ssh_config
# opkssh login opens the browser with xdg-open; this one hands the URL to Playwright.
cat >"$work/bin/xdg-open" <<EOF
#!/bin/sh
exec node "$helpers/browser.mjs" approve "\$1" "\${OPKSSH_E2E_PROFILE:-$profile}" >>"$work/approvals" 2>&1
EOF
chmod 755 "$work/bin/xdg-open"
touch "$work/approvals"
export PATH=$work/bin:$PATH
cert=$work/home/.ssh/corral-opkssh-cert.pub

# ContainerSSH 0.6.1's sshproxy backend ends a session as soon as the client's stdin ends, and
# can close it before relaying the command's last output. Hold stdin open, and let each command
# pause before it exits.
exec 3< <(exec sleep 3600)
pids+=($!)

login_as() {
  local user=$1
  shift
  HOME=$work/home ssh -F "$work/ssh_config" "$@" "$user@corral-e2e" \
    "id -un"$'\n''status=$?; sleep 0.2; exit $status' <&3 2>&1
}
refused() {
  local output
  if output=$(login_as "$@"); then
    fail "$1 let in [$*]"
  fi
  [[ "$output" == *"Permission denied"* ]] || fail "$*: unexpected error: $output"
}
approvals() {
  grep -c '^approved' "$work/approvals" || true
}

[[ ! -e "$cert" ]] || fail "a certificate before the first login"
expect "ssh with no certificate yet" sandbox "$(login_as auth-admin+main)"
[[ -s "$cert" ]] || fail "the hook wrote no certificate"
expect "browser logins" 1 "$(approvals)"
ok "with no certificate, the page's Match exec hook ran opkssh login, the browser step completed by itself, and ssh got in"

before=$(sha256sum <"$cert")
expect "ssh with a fresh certificate" sandbox "$(login_as auth-admin+main)"
expect "the certificate after a fresh ssh" "$before" "$(sha256sum <"$cert")"
expect "browser logins" 1 "$(approvals)"
ok "with a fresh certificate, ssh connects without logging in again"

touch -d '2 days ago' "$cert"
expect "ssh with a stale certificate" sandbox "$(login_as auth-admin+main)"
[[ "$(sha256sum <"$cert")" != "$before" ]] || fail "the stale certificate was not renewed"
expect "browser logins" 2 "$(approvals)"
ok "with a certificate older than its lifetime less an hour, the hook renewed it before connecting"

key=$work/home/.ssh/corral-opkssh
mv "$key" "$work/kept"
mv "$cert" "$work/kept-cert.pub"
denied_at=$SECONDS
OPKSSH_E2E_PROFILE=$work/no-session refused auth-admin+main
took=$((SECONDS - denied_at))
((took <= 45)) || fail "with no VoidAuth session, ssh took ${took}s"
expect "browser logins" 2 "$(approvals)"
ok "with no certificate and no VoidAuth session, the hook gives up and ssh is refused in ${took}s"
mv "$work/kept" "$key"
mv "$work/kept-cert.pub" "$cert"
plain_ssh() {
  local user=$1 identity=$2
  HOME=$work/home ssh -F /dev/null -p "$ssh_port" -i "$identity" -o IdentitiesOnly=yes \
    -o IdentityAgent=none -o BatchMode=yes -o PreferredAuthentications=publickey \
    -o "UserKnownHostsFile=$work/known_hosts" -o StrictHostKeyChecking=yes -o LogLevel=ERROR \
    "$user@127.0.0.1" "id -un"$'\n''status=$?; sleep 0.2; exit $status' <&3 2>&1
}
denied() {
  local output
  if output=$(plain_ssh "$1" "$2"); then
    fail "$1 let in with $2"
  fi
  [[ "$output" == *"Permission denied"* ]] || fail "$1 with $2: unexpected error: $output"
}

expect "the certificate without the hook" sandbox "$(plain_ssh auth-admin+main "$key")"
denied eve+main "$key"
ok "auth_admin's certificate is refused for eve+main, another user's sandbox"
denied nobody+main "$key"
ok "a certificate for a user Corral does not know is refused"

auth_admin_id=$(api GET api/admin/users | jq -r '.[] | select(.handle == "auth-admin") | .id')
api POST "api/admin/users/$auth_admin_id/disable" >/dev/null
denied auth-admin+main "$key"
api POST "api/admin/users/$auth_admin_id/enable" >/dev/null
expect "after enabling auth-admin again" sandbox "$(plain_ssh auth-admin+main "$key")"
ok "a disabled user's valid certificate is refused, and works once enabled"

cat >>"$work/home/.opk/config.yml" <<EOF
  - alias: other
    issuer: $issuer
    client_id: other
    scopes: openid profile email
    prompt: ""
EOF
(HOME=$work/home opkssh login other -i "$work/other" >>"$log" 2>&1)
denied auth-admin+main "$work/other"
ok "a real certificate for VoidAuth's other client (wrong audience) is refused"

sub=$(node "$helpers/mint.mjs" subject "$cert")
node "$helpers/mint.mjs" retoken "$cert" "$key" "$work/tampered" exp
denied auth-admin+main "$work/tampered"
ok "auth_admin's certificate with its token's exp changed, re-signed by its key, is refused"
ssh-keygen -q -t ecdsa -N '' -f "$work/thief"
node "$helpers/mint.mjs" retoken "$cert" "$work/thief" "$work/thief-cert" ""
denied auth-admin+main "$work/thief-cert"
ok "auth_admin's valid token in a certificate for another key is refused"

garbage=$(jq -cn --arg k "ecdsa-sha2-nistp256-cert-v01@openssh.com AAAAKGVjZHNh$(head -c 600 /dev/urandom | base64 -w0)" \
  '{username: "auth-admin+main", remoteAddress: "127.0.0.1:1", connectionId: "e2e", clientVersion: "SSH-2.0-e2e", publicKey: $k}')
answer=$(docker compose exec -T containerssh wget -q -O - --header 'Content-Type: application/json' \
  --post-data "$garbage" http://corral:8081/pubkey)
expect "/pubkey for a mangled certificate" false "$(jq -r .success <<<"$answer")"
expect "a login after the mangled certificate" sandbox "$(plain_ssh auth-admin+main "$key")"
ok "a mangled certificate gets success false from /pubkey, and the webhook keeps answering"

if docker compose logs --no-color corral 2>&1 | grep -qF "$(cut -d' ' -f2 "$cert" | head -c 80)"; then
  fail "a certificate appears in Corral's log"
fi
docker compose logs --no-color corral 2>&1 | grep -q 'opkssh refused a certificate.*audience does not contain clientID opkssh' \
  || fail "no logged reason for the wrong audience"
[[ -n "$(docker compose cp corral:/var/log/opkssh.log - | tar -xO)" ]] || fail "opkssh's log is empty"
[[ -n "$(docker compose cp corral:/var/cache/opkssh - | tar -t | grep -v '/$')" ]] || fail "no JWKS cache"
ok "Corral logs refusals by reason and never a certificate; opkssh logs to its file and caches the JWKS"

# A stand-in issuer mints what VoidAuth will not: old tokens and tokens from elsewhere. Corral is
# recreated to trust it, as uid 65532, the image's own user, with the Docker socket's group.
node "$helpers/mint.mjs" key "$work/mint"
mint_issuer=https://$gateway:$mint_port
node "$helpers/mint.mjs" serve "$gateway" "$mint_port" "$mint_issuer" "$work/mint" "$work/tls.pem" "$work/tls.key" \
  >>"$log" 2>&1 &
pids+=($!)
sed -i "s|^issuer = \"$issuer\"|issuer = \"$mint_issuer\"|" config.toml
socket_group=$(docker run --rm --entrypoint stat -v /var/run/docker.sock:/s "$sandbox_image" -c %g /s)
cat >compose.unprivileged.yaml <<EOF
services:
  corral:
    user: "65532:65532"
    group_add: ["$socket_group"]
EOF
docker compose stop corral >>"$log" 2>&1
docker run --rm --user 0 --entrypoint chown -v "${project}_corral-data:/data" "$sandbox_image" -R 65532:65532 /data
COMPOSE_FILE=$COMPOSE_FILE:compose.unprivileged.yaml docker compose up -d --wait --force-recreate corral >>"$log" 2>&1
expect "Corral's providers as uid 65532" "$mint_issuer opkssh 24h" \
  "$(docker compose cp corral:/etc/opk/providers - | tar -xO)"
ok "corral recreated as uid 65532 for the stand-in issuer $mint_issuer"

node "$helpers/mint.mjs" cert "$work/mint" "$mint_issuer" opkssh "$sub" 0 "$work/minted"
expect "a minted certificate" sandbox "$(plain_ssh auth-admin+main "$work/minted")"
ok "as uid 65532, a fresh token from the stand-in issuer lets auth_admin in: the minter matches opkssh"
node "$helpers/mint.mjs" cert "$work/mint" "$mint_issuer" opkssh "$sub" $((25 * 3600)) "$work/expired"
denied auth-admin+main "$work/expired"
ok "a token issued 25 hours ago is refused under the 24h policy"
node "$helpers/mint.mjs" cert "$work/mint" https://elsewhere.example opkssh "$sub" 0 "$work/elsewhere"
denied auth-admin+main "$work/elsewhere"
ok "a token from another issuer is refused"
[[ -n "$(docker compose cp corral:/var/cache/opkssh - | tar -t | grep -v '/$')" ]] || fail "no JWKS cache as 65532"
ok "as uid 65532, opkssh caches the JWKS"

echo "e2e opkssh: $passed checks passed in $((SECONDS - started))s"
