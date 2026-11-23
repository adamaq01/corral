#!/usr/bin/env bash
# Installs the chart into the kind cluster of crates/corral-engine-agent-sandbox/tests/kind/up.sh,
# which this script creates when it is missing, and walks a user and an operator through it.
# CORRAL_SERVER_IMAGE names a prebuilt server image (corral-server:e2e by default);
# CORRAL_SANDBOX_IMAGE a sandbox image (corral-sandbox:dev by default, built when missing).
# CORRAL_E2E_SKIP_NETWORK_POLICY=1 skips the NetworkPolicy check, for a cluster whose network
# plugin does not enforce policies.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
chart=$here/corral
node=corral-dev-control-plane
release=corral
namespace=corral-e2e
sandboxes=corral-e2e-sandboxes
http_port=18080
ssh_port=48291
forward_port=18000
base=http://127.0.0.1:$http_port
server_image=${CORRAL_SERVER_IMAGE:-corral-server:e2e}
export CORRAL_SANDBOX_IMAGE=${CORRAL_SANDBOX_IMAGE:-corral-sandbox:dev}
containerssh_image=containerssh/containerssh:$(awk '/^containerssh:/ { s = 1 } s && $1 == "tag:" { print $2; exit }' "$chart/values.yaml")

passed=0
work=
web_forward=
ssh_forward=
tunnel=
held=

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

remove_release() {
  helm uninstall "$release" --namespace "$namespace" --wait >/dev/null 2>&1 || true
  kubectl delete namespace "$namespace" "$sandboxes" --ignore-not-found --wait=true --timeout=180s >/dev/null 2>&1 || true
}

stop() {
  if [[ -n "$1" ]]; then
    kill "$1" 2>/dev/null || true
    wait "$1" 2>/dev/null || true
  fi
}

stop_forwards() {
  stop "$web_forward"
  stop "$ssh_forward"
  web_forward=
  ssh_forward=
}

cleanup() {
  local status=$?
  stop "$held"
  stop "$tunnel"
  stop_forwards
  if ((status != 0)); then
    kubectl --namespace "$namespace" get pods >&2 2>/dev/null || true
    kubectl --namespace "$namespace" logs deploy/$release --tail 40 >&2 2>/dev/null || true
    kubectl --namespace "$namespace" logs deploy/$release-containerssh --tail 20 >&2 2>/dev/null || true
    kubectl --namespace "$sandboxes" get sandboxes,pods,pvc >&2 2>/dev/null || true
  fi
  remove_release
  if [[ -n "$work" ]]; then
    rm -rf "$work"
  fi
  return "$status"
}
trap cleanup EXIT
trap 'echo "FAIL: line $LINENO: $BASH_COMMAND" >&2' ERR

for tool in kind kubectl helm kubeconform docker ssh ssh-keygen curl jq python3; do
  command -v "$tool" >/dev/null || fail "$tool is not on PATH"
done
work=$(mktemp -d "${TMPDIR:-/tmp}/corral-helm-e2e.XXXXXX")
log=$work/run.log

"$repo/crates/corral-engine-agent-sandbox/tests/kind/up.sh" >>"$log" 2>&1
kubectl config use-context kind-corral-dev >/dev/null
remove_release
load() {
  if ! kind load docker-image "$1" --name corral-dev >>"$log" 2>&1; then
    docker save "$1" | docker exec -i "$node" ctr --namespace=k8s.io images import - >>"$log"
  fi
}
docker image inspect "$containerssh_image" >/dev/null 2>&1 || docker pull -q "$containerssh_image" >>"$log"
load "$server_image"
load "$containerssh_image"
ok "kind cluster with agent-sandbox, images $server_image, $CORRAL_SANDBOX_IMAGE and $containerssh_image"

cat >"$work/values.yaml" <<EOF
baseUrl: $base
ssh:
  publicHost: 127.0.0.1
  publicPort: $ssh_port
image:
  repository: ${server_image%:*}
  tag: ${server_image##*:}
  pullPolicy: Never
sandboxes:
  namespace:
    name: $sandboxes
catalog:
  images:
    - key: dev
      label: Local build
      reference: $CORRAL_SANDBOX_IMAGE
idle:
  timeoutSecs: 30
  probeIntervalSecs: 5
keepOnUninstall: true
containerssh:
  service:
    type: NodePort
    nodePort: 30022
EOF

install() {
  helm upgrade --install "$release" "$chart" --namespace "$namespace" --create-namespace \
    --values "$work/values.yaml" "$@" --wait --timeout 3m >>"$log"
}

port_forward() {
  kubectl --namespace "$namespace" port-forward "svc/$1" "$2:$3" >>"$log" 2>&1 &
  forwarded=$!
  for _ in $(seq 50); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$2") 2>/dev/null; then
      return
    fi
    sleep 0.2
  done
  fail "port-forward to $1 never listened on $2"
}

# A port-forward stays on the pod it started with, so a new Corral pod needs a new one.
connect_web() {
  stop "$web_forward"
  port_forward "$release" "$http_port" 8080
  web_forward=$forwarded
}

connect() {
  connect_web
  stop "$ssh_forward"
  port_forward "$release-containerssh" "$ssh_port" "$ssh_port"
  ssh_forward=$forwarded
}

secret_data() {
  kubectl --namespace "$namespace" get secret "$1" -o jsonpath='{.data}'
}

install
connect
expect "the web UI's title" "<title>Corral</title>" "$(curl -sS --fail "$base/" | grep -o '<title>Corral</title>')"
keys=$(secret_data "$release-keys")
host_key=$(secret_data "$release-containerssh-host-key")
ok "helm install: Corral and ContainerSSH are ready and the web UI answers through the Service"

admin_password=$(head -c 18 /dev/urandom | base64)
kubectl --namespace "$namespace" exec -i "deploy/$release" -- \
  corral-server local-account create adam --admin <<<"$admin_password" >>"$log"
ok "kubectl exec deploy/$release -- corral-server local-account create adam --admin"

login() {
  jq -cn --arg p "$admin_password" '{username: "adam", password: $p}' \
    | curl -sS --fail-with-body -c "$work/jar" -H 'Content-Type: application/json' \
      --data @- "$base/auth/local/login" >/dev/null
}

api() {
  local args=(-sS --fail-with-body -X "$1" -b "$work/jar" -H 'Content-Type: application/json')
  if (($# > 2)); then
    args+=(--data "$3")
  fi
  curl "${args[@]}" "$base/$2"
}

login
expect "GET /api/me" "adam admin" "$(api GET api/me | jq -r '"\(.handle) \(.role)"')"
ssh-keygen -q -t ed25519 -N '' -C adam -f "$work/adam"
api POST api/keys "$(jq -cn --arg k "$(cat "$work/adam.pub")" '{label: "e2e", publicKey: $k}')" >/dev/null
expect "sandbox ssh username" "main.adam" "$(api POST api/sandboxes '{"name":"main"}' | jq -r .ssh.username)"
ok "adam signs in over the API, registers a key and creates sandbox main"

sandbox_id=$(kubectl --namespace "$sandboxes" get sandboxes -o jsonpath='{.items[0].metadata.name}')
kubectl get crd sandboxes.agents.x-k8s.io -o json \
  | jq '.spec.versions[] | select(.name == "v1beta1") | .schema.openAPIV3Schema' >"$work/sandbox_v1beta1.json"
summary=$(kubectl --namespace "$sandboxes" get sandbox "$sandbox_id" -o yaml \
  | kubeconform -strict -summary -schema-location "$work/{{.ResourceKind}}_{{.ResourceAPIVersion}}.json")
ok "Corral's Sandbox $sandbox_id against the installed agent-sandbox CRD schema: $summary"

kubectl --namespace "$namespace" get secret "$release-containerssh-host-key" \
  -o jsonpath='{.data.host_key}' | base64 -d >"$work/host_key"
chmod 600 "$work/host_key"
echo "[127.0.0.1]:$ssh_port $(ssh-keygen -y -f "$work/host_key")" >"$work/known_hosts"
cat >"$work/ssh_config" <<EOF
Host corral-e2e
  HostName 127.0.0.1
  Port $ssh_port
  IdentityFile $work/adam
  IdentitiesOnly yes
  IdentityAgent none
  BatchMode yes
  PreferredAuthentications publickey
  UserKnownHostsFile $work/known_hosts
  StrictHostKeyChecking yes
  ConnectTimeout 60
  LogLevel ERROR
EOF

as() {
  ssh -F "$work/ssh_config" "$1@corral-e2e" "$2" </dev/null
}

expect "ssh main.adam id -un" sandbox "$(as main.adam 'id -un')"
as main.adam 'echo kept >~/marker'
ok "ssh main.adam through the ContainerSSH Service runs as sandbox, the chart's host key pinned"

if ssh -F "$work/ssh_config" -o BatchMode=no -o PreferredAuthentications=password,keyboard-interactive \
  -o NumberOfPasswordPrompts=0 main.adam@corral-e2e true </dev/null >/dev/null 2>&1; then
  fail "a password login was let in"
fi
ok "password and keyboard-interactive logins are refused"

as main.adam 'nohup node -e "require(\"http\").createServer((q, r) => q.url === \"/hold\" ? r.writeHead(200).write(\"held\") : r.end(\"forwarded\")).listen(8000, \"127.0.0.1\")" >/dev/null 2>&1 &'
ssh -F "$work/ssh_config" -o ExitOnForwardFailure=yes -N \
  -L "127.0.0.1:$forward_port:127.0.0.1:8000" main.adam@corral-e2e </dev/null &
tunnel=$!
body=
for _ in $(seq 50); do
  body=$(curl -s "http://127.0.0.1:$forward_port/") && break
  sleep 0.2
done
expect "ssh -L" forwarded "$body"
curl -sN "http://127.0.0.1:$forward_port/hold" >/dev/null &
held=$!
ok "ssh -L $forward_port:127.0.0.1:8000 reaches a server inside the sandbox"

if [[ -z "${CORRAL_E2E_SKIP_NETWORK_POLICY:-}" ]]; then
  expect "Corral's web port from the sandbox" 200 \
    "$(as main.adam "curl -s -m 5 -o /dev/null -w %{http_code} http://$release.$namespace.svc:8080/healthz")"
  code=$(as main.adam "curl -s -m 5 -o /dev/null http://$release-webhooks.$namespace.svc:8081/healthz; echo \$?")
  [[ "$code" != 0 ]] || fail "the sandbox reached Corral's webhook port"
  ok "the NetworkPolicy keeps the sandbox off Corral's webhook port (curl exit $code)"
fi

state() {
  api GET api/sandboxes/main | jq -r .state
}
sleep 45
expect "main's state with a forwarded connection open" running "$(state)"
ok "an open forwarded connection keeps main awake 45s into a 30s idle timeout"

stop "$held"
stop "$tunnel"
held=
tunnel=
kubectl --namespace "$sandboxes" exec "$sandbox_id" -c sandbox -- \
  pgrep -f 'containerssh-agent forward-server' >/dev/null || fail "no port-forward agent was left behind"
waited=0
until [[ "$(state)" == sleeping ]]; do
  ((waited < 120)) || fail "main was still $(state) after ${waited}s"
  sleep 5
  waited=$((waited + 5))
done
kubectl --namespace "$sandboxes" wait --for=delete "pod/$sandbox_id" --timeout=60s >>"$log"
ok "with the forwarded connection closed and its agent left behind, main slept within ${waited}s"

woke_at=$SECONDS
expect "the home after a wake" kept "$(as main.adam 'cat ~/marker')"
ok "ssh woke main in $((SECONDS - woke_at))s, home kept"

pod() {
  kubectl --namespace "$namespace" get pods -l app.kubernetes.io/component=server -o jsonpath='{.items[0].metadata.uid}'
}
before=$(pod)
as main.adam 'sleep 30; echo survived' >"$work/session.out" &
session=$!
sleep 2
install --set limits.maxRunning=7
connect_web
[[ "$(pod)" != "$before" ]] || fail "the config changed and Corral was not restarted"
wait "$session" || true
expect "an SSH session open across the upgrade" survived "$(cat "$work/session.out")"
expect "the keys Secret after an upgrade" "$keys" "$(secret_data "$release-keys")"
expect "the host key Secret after an upgrade" "$host_key" "$(secret_data "$release-containerssh-host-key")"
login
expect "keys after the upgrade" 1 "$(api GET api/keys | jq length)"
expect "the home after the upgrade" kept "$(as main.adam 'cat ~/marker')"
ok "helm upgrade restarted Corral with new config; an open SSH session, data, generated keys and host key survived"

stop_forwards
helm uninstall "$release" --namespace "$namespace" --wait >>"$log"
expect "kept in $namespace" "persistentvolumeclaim/$release-data secret/$release-containerssh-host-key secret/$release-keys" \
  "$(kubectl --namespace "$namespace" get pvc,secrets -o name | sort | xargs)"
expect "kept in $sandboxes" "sandbox.agents.x-k8s.io/$sandbox_id" \
  "$(kubectl --namespace "$sandboxes" get sandboxes -o name)"
ok "helm uninstall with keepOnUninstall: the database volume, keys, host key and sandboxes stay"

install
connect
expect "the keys Secret after a reinstall" "$keys" "$(secret_data "$release-keys")"
expect "the host key Secret after a reinstall" "$host_key" "$(secret_data "$release-containerssh-host-key")"
login
expect "sandboxes after a reinstall" main "$(api GET api/sandboxes | jq -r '.[].name')"
expect "the home after a reinstall" kept "$(as main.adam 'cat ~/marker')"
ok "helm install over the kept data: same admin, same sandbox and home, same host key"

install --set keepOnUninstall=false
stop_forwards
helm uninstall "$release" --namespace "$namespace" --wait >>"$log"
kubectl wait --for=delete "namespace/$sandboxes" --timeout=180s >>"$log" 2>&1
expect "left in $namespace" "secret/$release-containerssh-host-key secret/$release-keys" \
  "$(kubectl --namespace "$namespace" get pvc,secrets -o name | sort | xargs)"
ok "helm uninstall without keepOnUninstall deletes the database volume and the sandbox namespace; the generated keys stay"

echo "all $passed checks passed"
