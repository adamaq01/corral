#!/usr/bin/env bash
# CORRAL_SERVER_IMAGE and CORRAL_SANDBOX_IMAGE name prebuilt images; when unset, both are built
# from images/.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
project=corral-e2e
# Its own network and ports, so a run never touches a real install on the same host.
sandbox_network=corral-e2e-sandboxes
http_port=18080
ssh_port=48292
forward_port=18000
base=http://127.0.0.1:$http_port
# Sandboxes reach the API through the host, at their network's gateway, as they would through
# the host's LAN address in a real install.
sandbox_subnet=172.30.243.0/24
sandbox_gateway=172.30.243.1
sandbox_api_port=18081

started=$SECONDS
passed=0
work=

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

hc_local() {
  HOME=$work/hc-home XDG_RUNTIME_DIR=$work/hc-run hc "$@"
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
  docker network rm "$sandbox_network" >/dev/null 2>&1 || true
}

cleanup() {
  local status=$?
  if [[ -n "$work" && -d "$work/hc-home" ]]; then
    hc_local server stop >/dev/null 2>&1 || true
  fi
  if ((status != 0)) && [[ -n "$work" ]]; then
    tail -n 60 "$work/run.log" >&2 2>/dev/null || true
    (cd "$work" && docker compose logs --no-color --tail 40 >&2 2>&1) || true
  fi
  remove_stack || true
  if [[ -n "$work" ]]; then
    rm -rf "$work"
  fi
  return "$status"
}
trap cleanup EXIT
trap 'echo "FAIL: line $LINENO: $BASH_COMMAND" >&2' ERR

for tool in docker ssh ssh-keygen curl jq hc; do
  command -v "$tool" >/dev/null || fail "$tool is not on PATH"
done
remove_stack
work=$(mktemp -d "${TMPDIR:-/tmp}/corral-e2e.XXXXXX")
log=$work/run.log

server_image=${CORRAL_SERVER_IMAGE:-corral-server:e2e}
sandbox_image=${CORRAL_SANDBOX_IMAGE:-corral-sandbox:dev}
if [[ -z "${CORRAL_SERVER_IMAGE:-}" ]]; then
  docker build --file "$repo/images/server/Dockerfile" --tag "$server_image" "$repo" >>"$log" 2>&1
fi
if [[ -z "${CORRAL_SANDBOX_IMAGE:-}" ]]; then
  docker build --file "$repo/images/sandbox/Dockerfile" --tag "$sandbox_image" "$repo" >>"$log" 2>&1
fi
ok "images: server $server_image, sandbox $sandbox_image"

cp -R "$repo/deploy/compose/." "$work/"
cd "$work"
chmod 700 .
sed -e "s|^base_url = .*|base_url = \"$base\"\nsandbox_base_url = \"http://$sandbox_gateway:$sandbox_api_port\"|" \
  -e 's|^public_host = .*|public_host = "127.0.0.1"|' \
  "$repo/deploy/compose/config.toml" >config.toml
cat >>config.toml <<EOF
public_port = $ssh_port

[engine]
kind = "docker"
network = "$sandbox_network"

[idle]
probe_interval_secs = 1

[limits]
max_sandboxes_per_user = 2

[[catalog.images]]
key = "dev"
label = "Local build"
reference = "$sandbox_image"
EOF
cat >compose.e2e.yaml <<EOF
services:
  corral:
    image: $server_image
    ports: !override
      - "127.0.0.1:$http_port:8080"
      - "$sandbox_gateway:$sandbox_api_port:8080"
  containerssh:
    ports: !override
      - "127.0.0.1:$ssh_port:2222"
networks:
  corral-sandboxes:
    name: $sandbox_network
    ipam:
      config:
        - subnet: $sandbox_subnet
          gateway: $sandbox_gateway
EOF
export COMPOSE_PROJECT_NAME=$project COMPOSE_FILE=compose.yaml:compose.e2e.yaml
ok "copied deploy/compose to a private directory and set base_url and public_host"

docker compose run --rm corral keys generate >.env 2>>"$log"
expect "keys generate" "CORRAL_SESSION_KEY CORRAL_SECRETS_KEY" "$(cut -d= -f1 .env | xargs)"
ok "docker compose run --rm corral keys generate > .env wrote the two keys"

for key in containerssh_host_key containerssh_proxy_key; do
  ssh-keygen -q -t ed25519 -N '' -C containerssh -f "$key" && chmod 644 "$key"
done
echo "CORRAL_PROXY_PUBLIC_KEY=$(cat containerssh_proxy_key.pub)" >>.env
ok "generated ContainerSSH's host and proxy keys, and gave Corral the proxy's public key"

docker compose up -d --wait >>"$log" 2>&1
ok "docker compose up -d --wait: corral and containerssh are healthy"

page=$(curl -sS --fail-with-body -D "$work/ui.headers" "$base/sandboxes/main")
[[ "$page" == *"<title>Corral</title>"* ]] || fail "GET /sandboxes/main is not the web UI: $page"
grep -qi "^content-security-policy: .*script-src 'self' 'sha256-" "$work/ui.headers" \
  || fail "the web UI's CSP allows no inline script: $(cat "$work/ui.headers")"
expect "GET /api/nope" '{"error":"not found"}' "$(curl -sS "$base/api/nope")"
ok "the image serves the web UI at a deep link with its CSP, and the API's JSON 404s"

admin_password=$(head -c 18 /dev/urandom | base64)
docker compose exec corral corral-server local-account create adam --admin <<<"$admin_password" >>"$log"
ok "docker compose exec corral corral-server local-account create adam --admin"

login() {
  jq -cn --arg u "$1" --arg p "$2" '{username: $u, password: $p}' \
    | curl -sS --fail-with-body -c "$work/$1.jar" -H 'Content-Type: application/json' \
      --data @- "$base/auth/local/login" >/dev/null
}

api() {
  local args=(-sS --fail-with-body -X "$2" -b "$work/$1.jar" -H 'Content-Type: application/json')
  if (($# > 3)); then
    args+=(--data "$4")
  fi
  curl "${args[@]}" "$base/$3"
}

http_status() {
  curl -sS -o /dev/null -w '%{http_code}' -X "$2" -b "$work/$1.jar" "$base/$3"
}

add_key() {
  api "$1" POST api/keys "$(jq -cn --arg k "$(cat "keys/$1.pub")" '{label: "e2e", publicKey: $k}')" >/dev/null
}

expect "GET /auth/providers" local "$(curl -sS --fail-with-body "$base/auth/providers" | jq -r '.[].id')"
login adam "$admin_password"
expect "GET /api/me" "adam admin" "$(api adam GET api/me | jq -r '"\(.handle) \(.role)"')"
ok "adam logs in with POST /auth/local/login"

mkdir -m 700 keys
for name in adam bob stranger; do
  ssh-keygen -q -t ed25519 -N '' -C "$name" -f "keys/$name"
done
add_key adam
expect "sandbox ssh username" "main.adam" \
  "$(api adam POST api/sandboxes '{"name":"main"}' | jq -r .ssh.username)"
adam_container=$(docker ps --format '{{.Names}}' --filter "network=$sandbox_network" \
  --filter label=corral.adamaq01.fr/owner=adam)
[[ -n "$adam_container" ]] || fail "no container for adam's sandbox on $sandbox_network"
adam_home=$(docker inspect --format '{{range .Mounts}}{{.Name}}{{end}}' "$adam_container")
ok "adam registered a key and created sandbox main ($adam_container)"

echo "[127.0.0.1]:$ssh_port $(cut -d' ' -f1,2 containerssh_host_key.pub)" >known_hosts
cat >ssh_config <<EOF
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
as() {
  ssh -n -F "$work/ssh_config" -i "$work/keys/$1" "$2@corral-e2e" "$3"
}

refused() {
  local output
  if output=$(as "$1" "$2" true 2>&1); then
    fail "$2 let $1's key in"
  fi
  [[ "$output" == *"Permission denied"* ]] || fail "$2 with $1's key: unexpected error: $output"
}

expect "ssh -n main.adam id -un" sandbox "$(as adam main.adam "id -un")"
as adam main.adam 'echo adam >~/owner'
ok "ssh -n main.adam@127.0.0.1 -p $ssh_port runs as sandbox, ContainerSSH's host key pinned"

expect "bytes from ssh -n main.adam with a 200 kB output" 200000 \
  "$(as adam main.adam 'head -c 200000 /dev/zero | tr "\0" a' | wc -c | xargs)"
ok "a one-shot command's 200 kB output arrives whole with stdin closed"

printf '#!/bin/sh\ntouch "%s/asked"\necho guess\n' "$work" >askpass
chmod 700 askpass
if output=$(SSH_ASKPASS=$work/askpass SSH_ASKPASS_REQUIRE=force ssh -F ssh_config -o BatchMode=no \
  -o PreferredAuthentications=password,keyboard-interactive -o NumberOfPasswordPrompts=1 \
  main.adam@corral-e2e true 2>&1 </dev/null); then
  fail "a password login was let in"
fi
[[ "$output" == *"Permission denied"* ]] || fail "password login: unexpected error: $output"
[[ -e asked ]] || fail "ssh never offered a password"
ok "password and keyboard-interactive logins are refused"

refused stranger main.adam
ok "an unregistered key is refused"

expect "ssh scratch.adam before scratch exists" sandbox "$(as adam scratch.adam "id -un")"
expect "scratch's image and size" "dev small" \
  "$(api adam GET api/sandboxes/scratch | jq -r '"\(.image) \(.size)"')"
ok "ssh scratch.adam made sandbox scratch from adam's defaults, then opened it"

expect "ssh ssh://main.adam@corral-e2e:$ssh_port" sandbox \
  "$(ssh -n -F ssh_config -i keys/adam "ssh://main.adam@corral-e2e:$ssh_port" 'id -un')"
ok "an ssh:// URI takes main.adam as written, as hc machine add passes it"

refused adam adam+main
refused adam Scratch_2.adam
ok "adam+main and an invalid sandbox name are refused"

if output=$(as adam extra.adam true 2>&1); then
  fail "extra.adam let adam past his limit of 2 sandboxes"
fi
[[ "$output" == *"closed by remote host"* ]] || fail "extra.adam: unexpected error: $output"
expect "adam's sandboxes at his limit" "main scratch" \
  "$(api adam GET api/sandboxes | jq -r '.[].name' | xargs)"
ok "ssh extra.adam past adam's limit is refused and makes nothing"
api adam DELETE api/sandboxes/scratch >/dev/null

bob_password=$(head -c 18 /dev/urandom | base64)
api adam POST auth/local/accounts \
  "$(jq -cn --arg p "$bob_password" '{username: "bob", password: $p, role: "member"}')" >/dev/null
login bob "$bob_password"
add_key bob
api bob POST api/sandboxes '{"name":"main"}' >/dev/null
bob_home=$(docker inspect --format '{{range .Mounts}}{{.Name}}{{end}}' \
  "$(docker ps --quiet --filter "network=$sandbox_network" --filter label=corral.adamaq01.fr/owner=bob)")
expect "ssh main.bob id -un" sandbox "$(as bob main.bob "id -un")"
expect "bob's home" none "$(as bob main.bob 'cat ~/owner 2>/dev/null || echo none')"
ok "bob, added with the admin API, reaches his own sandbox and home"

refused adam main.bob
ok "adam's key is refused for main.bob"

mkdir -m 700 hc-home hc-run
# ssh reads ~/.ssh/config from the passwd home, not $HOME, so hc gets the e2e's ssh command.
hc_local machine add --name e2e --install \
  --via "ssh -F $work/ssh_config -i $work/keys/adam main.adam@corral-e2e" >>"$log"
expect "hc exec --on e2e echo ok" ok "$(hc_local exec --on e2e echo ok)"
hc_local server stop >>"$log"
ok "hc machine add over ContainerSSH, then hc exec --on e2e echo ok prints ok"

as adam main.adam 'nohup node -e "require(\"http\").createServer((q, r) => r.end(\"forwarded\")).listen(8000, \"127.0.0.1\")" </dev/null >/dev/null 2>&1 &'
inside=
for _ in $(seq 50); do
  inside=$(as adam main.adam 'curl -s http://127.0.0.1:8000/') && break
  sleep 0.2
done
expect "the server inside the sandbox" forwarded "$inside"
ssh -F ssh_config -i keys/adam -o ExitOnForwardFailure=yes -N \
  -L "127.0.0.1:$forward_port:127.0.0.1:8000" main.adam@corral-e2e &
forward=$!
body=
for _ in $(seq 50); do
  body=$(curl -s "http://127.0.0.1:$forward_port/") && break
  sleep 0.2
done
kill "$forward" 2>/dev/null || true
wait "$forward" 2>/dev/null || true
expect "ssh -L" forwarded "$body"
ok "ssh -L $forward_port:127.0.0.1:8000 reaches a server inside the sandbox"

corral_ip=$(docker inspect --format \
  "{{(index .NetworkSettings.Networks \"${project}_corral\").IPAddress}}" "$project-corral-1")
codes=
for url in http://corral:8081/healthz "http://$corral_ip:8081/healthz" "http://$corral_ip:8080/healthz"; do
  code=$(as adam main.adam "curl -s -m 5 -o /dev/null $url; echo \$?")
  [[ "$code" =~ ^[0-9]+$ && "$code" != 0 ]] || fail "from the sandbox, $url answered (curl exit [$code])"
  codes+=" $code"
done
ok "from the sandbox, Corral is unreachable by name and at $corral_ip (curl exits:$codes)"

api adam POST api/sandboxes/main/stop >/dev/null
expect "container after stop" exited "$(docker inspect --format '{{.State.Status}}' "$adam_container")"
expect "state after stop" sleeping "$(api adam GET api/sandboxes/main | jq -r .state)"
token="e2e-$(head -c 12 /dev/urandom | base64 | tr -d '/+=')"
api adam PUT api/credentials/E2E_TOKEN "$(jq -cn --arg v "$token" '{value: $v}')" >/dev/null
listed=$(api adam GET api/credentials)
expect "GET /api/credentials" E2E_TOKEN "$(jq -r '.[].name' <<<"$listed")"
[[ "$listed" != *"$token"* ]] || fail "GET /api/credentials returned the value"
woke_at=$SECONDS
expect "after the wake" "$token adam" "$(as adam main.adam 'echo "$E2E_TOKEN $(cat ~/owner)"')"
ok "a credential set while main slept reached SSH on the wake ($((SECONDS - woke_at))s), home kept"

if docker compose logs --no-color 2>&1 | grep -qF "$token"; then
  fail "the credential value appears in the service logs"
fi
ok "the credential value is in neither service's logs"

bob_id=$(api adam GET api/admin/users | jq -r '.[] | select(.handle == "bob") | .id')
expect "bob's status once disabled" disabled \
  "$(api adam POST "api/admin/users/$bob_id/disable" | jq -r .status)"
refused bob main.bob
expect "disabled bob's GET /api/me" 403 "$(http_status bob GET api/me)"
ok "a disabled user is refused over SSH and by the API"

docker compose restart corral >>"$log" 2>&1
docker compose up -d --wait >>"$log" 2>&1
login adam "$admin_password"
expect "keys after the restart" 1 "$(api adam GET api/keys | jq length)"
expect "sandboxes after the restart" main "$(api adam GET api/sandboxes | jq -r '.[].name')"
expect "credentials after the restart" E2E_TOKEN "$(api adam GET api/credentials | jq -r '.[].name')"
expect "ssh after the restart" sandbox "$(as adam main.adam "id -un")"
api adam POST api/sandboxes/main/restart >/dev/null
expect "the credential after an API restart" "$token" "$(as adam main.adam 'printenv E2E_TOKEN')"
ok "after docker compose restart corral, the user, key, sandbox and credential persist"

started_at() {
  docker inspect --format '{{.State.StartedAt}}' "$adam_container"
}

until_state() {
  local waited=0
  while [[ "$(api adam GET api/sandboxes/main | jq -r .state)" != "$1" ]]; do
    ((waited < 60)) || fail "main was not $1 after 60s"
    sleep 1
    waited=$((waited + 1))
  done
}

out=$(as adam main.adam 'corral status')
[[ "$out" == *"Sandbox  main.adam"*"State    running"* ]] || fail "corral status: $out"
ok "corral status inside main shows main.adam running"

before=$(started_at)
out=$(as adam main.adam 'corral restart' 2>&1 || true)
[[ "$out" == "Corral accepted the restart of main."* ]] || fail "corral restart: $out"
waited=0
while [[ "$(started_at)" == "$before" || "$(api adam GET api/sandboxes/main | jq -r .state)" != running ]]; do
  ((waited < 60)) || fail "main did not come back within 60s of corral restart"
  sleep 1
  waited=$((waited + 1))
done
expect "ssh after corral restart" "sandbox $token" "$(as adam main.adam 'echo "$(id -un) $E2E_TOKEN"')"
ok "corral restart inside main was accepted, and main came back (${waited}s)"

out=$(as adam main.adam 'corral stop' 2>&1 || true)
[[ "$out" == "Corral accepted the stop of main."* ]] || fail "corral stop: $out"
until_state sleeping
expect "container after corral stop" exited "$(docker inspect --format '{{.State.Status}}' "$adam_container")"
ok "corral stop inside main was accepted, and main went to sleep"

out=$(as adam main.adam 'corral delete 2>&1; echo "exit $?"')
[[ "$out" == *"To go ahead, run: corral delete --confirm main"*"exit 1" ]] || fail "corral delete: $out"
out=$(as adam main.adam 'corral delete --confirm main 2>&1; echo "exit $?"')
[[ "$out" == *"deleting a sandbox from inside it is off"*"exit 1" ]] \
  || fail "corral delete --confirm while off: $out"
expect "deleteAt while off" null "$(api adam GET api/sandboxes/main | jq -r .deleteAt)"
ok "corral delete asks for --confirm, then is refused while adam's profile has it off"

api adam PUT api/me/delete-from-inside '{"enabled":true}' >/dev/null
out=$(as adam main.adam 'corral delete --confirm main' 2>&1 || true)
[[ "$out" == "main stops now and is deleted at "*"cancel the deletion on its page: $base/sandboxes/main" ]] \
  || fail "corral delete --confirm main: $out"
until_state sleeping
delete_at=$(api adam GET api/sandboxes/main | jq -r .deleteAt)
[[ "$delete_at" != null ]] || fail "no deleteAt after corral delete"
if output=$(as adam main.adam true 2>&1); then
  fail "ssh woke main while its deletion was scheduled"
fi
[[ "$output" == *"closed by remote host"* ]] || fail "ssh to a scheduled main: unexpected error: $output"
expect "container while scheduled" exited "$(docker inspect --format '{{.State.Status}}' "$adam_container")"
ok "with it on, corral delete --confirm main stopped main and scheduled its deletion for $delete_at"

docker compose restart corral >>"$log" 2>&1
docker compose up -d --wait >>"$log" 2>&1
login adam "$admin_password"
expect "deleteAt after a restart of corral" "$delete_at" "$(api adam GET api/sandboxes/main | jq -r .deleteAt)"
expect "cancel the deletion" 204 "$(http_status adam DELETE api/sandboxes/main/deletion)"
expect "deleteAt after the cancel" null "$(api adam GET api/sandboxes/main | jq -r .deleteAt)"
expect "ssh after the cancel" "sandbox adam" "$(as adam main.adam 'echo "$(id -un) $(cat ~/owner)"')"
ok "the schedule outlived a restart of corral; cancelling it through the API kept main and its home"

docker compose restart containerssh >>"$log" 2>&1
docker compose up -d --wait >>"$log" 2>&1
expect "ssh after a ContainerSSH restart" sandbox "$(as adam main.adam "id -un")"
ok "after docker compose restart containerssh, the pinned host key still matches"

for _ in 1 2 3 4 5; do
  expect "a wrong password" 403 "$(curl -sS -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' \
    --data '{"username":"adam","password":"not the password"}' "$base/auth/local/login")"
done
expect "the right password after 5 wrong ones" 429 "$(jq -cn --arg p "$admin_password" \
  '{username: "adam", password: $p}' | curl -sS -o /dev/null -w '%{http_code}' \
  -H 'Content-Type: application/json' --data @- "$base/auth/local/login")"
ok "after 5 wrong passwords, POST /auth/local/login answers 429 even to the right one"

# Short timings for the rest: a wake deadline a slow wake can exceed, and a quick idle timeout.
# The bind mount follows config.toml's inode, so it is rewritten in place.
awk '{ print } /^public_port = / { print "wake_deadline_secs = 2" }
  /^probe_interval_secs = / { print "timeout_secs = 3" }' config.toml >config.new
cat config.new >config.toml
docker compose restart corral >>"$log" 2>&1
docker compose up -d --wait >>"$log" 2>&1
login adam "$admin_password"
api adam POST api/sandboxes '{"name":"idle"}' >/dev/null
idle=$(docker ps --format '{{.Names}}' --filter "network=$sandbox_network" \
  --filter label=corral.adamaq01.fr/sandbox=idle)
idle_home=$(docker inspect --format '{{range .Mounts}}{{.Name}}{{end}}' "$idle")
ok "restarted corral with wake_deadline_secs = 2 and timeout_secs = 3, created sandbox idle"

state() {
  docker inspect --format '{{.State.Status}}' "$idle"
}

sleeps_within() {
  local waited=0
  while [[ "$(state)" != exited ]]; do
    ((waited < $1)) || fail "$idle was still $(state) after ${1}s"
    sleep 1
    waited=$((waited + 1))
  done
  echo "$waited"
}

as adam idle.adam 'sleep 6'
expect "after a 6s SSH session" running "$(state)"
ok "an SSH session longer than the idle timeout keeps idle awake"
slept=$(sleeps_within 15)
ok "with nothing running, idle slept ${slept}s after the session ended"

as adam idle.adam 'sudo mv /usr/bin/pgrep /usr/bin/pgrep.off'
sleep 6
expect "6s after breaking the probes" running "$(state)"
if ! docker compose logs --no-color corral 2>&1 | grep "activity probe failed, keeping the sandbox awake" \
  | grep -qF "${idle#corral-}"; then
  fail "no log line for idle's failed activity probe"
fi
as adam idle.adam 'sudo mv /usr/bin/pgrep.off /usr/bin/pgrep'
slept=$(sleeps_within 15)
ok "a failing activity probe keeps idle awake; once fixed, it slept ${slept}s later"

as adam idle.adam 'hc run --bg "sleep 300" >/dev/null 2>&1'
sleep 6
expect "6s after starting a Herdr terminal" running "$(state)"
as adam idle.adam 'sudo mv /usr/local/bin/hc /usr/local/bin/hc.off'
slept=$(sleeps_within 15)
ok "a live Herdr terminal keeps idle awake; with hc missing (exit 127) it slept ${slept}s later"

# main keeps the default idle timeout, so the reaper leaves its slow wake alone. Starved of CPU,
# it cannot wake within the deadline; the CPU comes back while ContainerSSH waits to retry.
api adam POST api/sandboxes/main/stop >/dev/null
cpus=$(docker inspect --format '{{.HostConfig.NanoCpus}}' "$adam_container")
docker update --cpus 0.01 "$adam_container" >/dev/null
called=$(date +%s)
answer=$(docker compose exec containerssh wget -q -O /dev/null --header 'Content-Type: application/json' \
  --post-data '{"username":"main.adam","authenticatedUsername":"main.adam","remoteAddress":"127.0.0.1:1","connectionId":"e2e","clientVersion":"SSH-2.0-e2e"}' \
  http://corral:8081/config 2>&1 || true)
answered=$(($(date +%s) - called))
[[ "$answer" == *" 503 "* ]] || fail "/config during a slow wake answered: $answer"
((answered <= 5)) || fail "/config took ${answered}s to answer 503"
woke_at=$SECONDS
as adam main.adam "id -un" >slow-login.out &
slow_login=$!
sleep 3
docker update --cpus "$(awk -v n="$cpus" 'BEGIN { print n / 1e9 }')" "$adam_container" >/dev/null
wait "$slow_login" || true
took=$((SECONDS - woke_at))
expect "ssh into the slow sandbox" sandbox "$(cat slow-login.out)"
((took >= 10)) || fail "ssh got in after ${took}s, before any ContainerSSH retry"
ok "a wake past the deadline: /config answers 503 in ${answered}s, ssh gets in on ContainerSSH's retry (${took}s)"

api adam POST "api/admin/users/$bob_id/enable" >/dev/null
login bob "$bob_password"
api bob PUT api/credentials/BOB_TOKEN '{"value":"bob-secret"}' >/dev/null
expect "bob's sandboxes, keys and credentials" "1 1 1" "$(for listing in sandboxes keys credentials; do
  api bob GET "api/$listing" | jq length; done | xargs)"
api adam DELETE "api/admin/users/$bob_id" >/dev/null
expect "bob's containers after deleting bob" "" \
  "$(docker ps --all --quiet --filter "network=$sandbox_network" --filter label=corral.adamaq01.fr/owner=bob)"
if docker volume inspect "$bob_home" >/dev/null 2>&1; then
  fail "$bob_home survived deleting bob"
fi
refused bob main.bob
expect "deleted bob's GET /api/me" 403 "$(http_status bob GET api/me)"
expect "bob in the user list" "" "$(api adam GET api/admin/users | jq -r '.[] | select(.handle == "bob") | .id')"
ok "deleting bob removed his container and home, and his key is refused over SSH"

api adam POST auth/local/accounts \
  "$(jq -cn --arg p "$bob_password" '{username: "bob", password: $p, role: "member"}')" >/dev/null
login bob "$bob_password"
expect "the new bob's sandboxes, keys and credentials" "0 0 0" "$(for listing in sandboxes keys credentials; do
  api bob GET "api/$listing" | jq length; done | xargs)"
new_bob_id=$(api adam GET api/admin/users | jq -r '.[] | select(.handle == "bob") | .id')
[[ -n "$new_bob_id" && "$new_bob_id" != "$bob_id" ]] || fail "the new bob reused id [$new_bob_id]"
refused bob main.bob
ok "recreating the local account bob gives a new, empty user"

api adam DELETE api/sandboxes/main >/dev/null
api adam DELETE api/sandboxes/idle >/dev/null
expect "sandbox containers after the deletes" "" \
  "$(docker ps --all --quiet --filter "network=$sandbox_network" --filter label=app.kubernetes.io/managed-by=corral)"
for volume in "$adam_home" "$idle_home"; do
  if docker volume inspect "$volume" >/dev/null 2>&1; then
    fail "$volume survived its sandbox's delete"
  fi
done
ok "deleting the sandboxes through the API removed their containers and homes"

docker compose down --volumes >>"$log" 2>&1
expect "project containers after down" "" \
  "$(docker ps --all --quiet --filter "label=com.docker.compose.project=$project")"
if docker network inspect "$sandbox_network" >/dev/null 2>&1; then
  fail "docker compose down left $sandbox_network"
fi
ok "docker compose down --volumes removed the stack and both networks"

echo "e2e: $passed checks passed in $((SECONDS - started))s"
