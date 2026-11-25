#!/usr/bin/env bash
# Installs the chart into the kind cluster of crates/corral-engine-agent-sandbox/tests/kind/up.sh,
# which this script creates when it is missing, and walks a user and an operator through it:
# deploy/helm/e2e.sh [smoke|all], or CORRAL_E2E_PART. all adds the chart's lifecycle: an upgrade,
# an uninstall that keeps the data, a reinstall over it, and an uninstall that does not keep it.
# CORRAL_SERVER_IMAGE, CORRAL_SANDBOX_IMAGE and CORRAL_OVERLAY_IMAGE name prebuilt images; when
# unset, this checkout's are built. The cluster and the ports are CORRAL_RUN_ID's (tests/checkout.sh).
# CORRAL_E2E_SKIP_NETWORK_POLICY=1 skips the NetworkPolicy check, for a cluster whose network
# plugin does not enforce policies.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
source "$repo/tests/checkout.sh"
export KUBECONFIG=$CORRAL_KUBECONFIG
part=${1:-${CORRAL_E2E_PART:-smoke}}
if [[ "$part" != smoke && "$part" != all ]]; then
  echo "usage: deploy/helm/e2e.sh [smoke|all]" >&2
  exit 2
fi
chart=$here/corral
cluster=corral-$CORRAL_RUN_ID
node=$cluster-control-plane
release=corral
namespace=corral-e2e
sandboxes=corral-e2e-sandboxes
http_port=$(corral_port 9)
ssh_port=$(corral_port 10)
forward_port=$(corral_port 11)
base=http://127.0.0.1:$http_port
server_image=$CORRAL_SERVER_IMAGE

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
    kubectl --namespace "$namespace" logs deploy/$release-gate --tail 20 >&2 2>/dev/null || true
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
"$repo/tests/images.sh" server >>"$log" 2>&1
remove_release
load() {
  if ! kind load docker-image "$1" --name "$cluster" >>"$log" 2>&1; then
    docker save "$1" | docker exec -i "$node" ctr --namespace=k8s.io images import - >>"$log"
  fi
}
load "$server_image"
ok "kind cluster with agent-sandbox, images $server_image and $CORRAL_SANDBOX_IMAGE"

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
  overlayImage:
    repository: ${CORRAL_OVERLAY_IMAGE%:*}
    tag: ${CORRAL_OVERLAY_IMAGE##*:}
catalog:
  images:
    - key: dev
      label: Local build
      reference: $CORRAL_SANDBOX_IMAGE
idle:
  timeoutSecs: 30
  checkIntervalSecs: 5
  reportIntervalSecs: 5
keepOnUninstall: true
gate:
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
  port_forward "$release-gate" "$ssh_port" "$ssh_port"
  ssh_forward=$forwarded
}

secret_data() {
  kubectl --namespace "$namespace" get secret "$1" -o jsonpath='{.data}'
}

install
connect
expect "the web UI's title" "<title>Corral</title>" "$(curl -sS --fail "$base/" | grep -o '<title>Corral</title>')"
keys=$(secret_data "$release-keys")
host_key=$(secret_data "$release-ssh-host-key")
ok "helm install: Corral and the gate are ready and the web UI answers through the Service"

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

kubectl --namespace "$namespace" get secret "$release-ssh-host-key" \
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
ok "ssh main.adam through the gate's Service runs as sandbox, the chart's host key pinned"

if ssh -F "$work/ssh_config" -o BatchMode=no -o PreferredAuthentications=password,keyboard-interactive \
  -o NumberOfPasswordPrompts=0 main.adam@corral-e2e true </dev/null >/dev/null 2>&1; then
  fail "a password login was let in"
fi
ok "password and keyboard-interactive logins are not offered"

# The command is typed before ssh connects, so it reaches the session before the shell starts.
type_ahead_and_drop() {
  python3 -I -c '
import os, pty, select, sys, time
pid, fd = pty.fork()
if pid == 0:
    os.execvp(sys.argv[1], sys.argv[1:])
os.write(fd, b"echo typed-$((6 * 7))\n")
seen = b""
end = time.time() + 30
while b"typed-42\r\n" not in seen and time.time() < end:
    if select.select([fd], [], [], 0.5)[0]:
        try:
            seen += os.read(fd, 4096)
        except OSError:
            break
os.kill(pid, 9)
os.waitpid(pid, 0)
print(repr(seen))
' ssh -F "$work/ssh_config" main.adam@corral-e2e
}
shells() {
  kubectl --namespace "$sandboxes" exec "$sandbox_id" -c sandbox -- pgrep -c -x bash 2>/dev/null || true
}
shells_before=$(shells)
typed=$(type_ahead_and_drop)
[[ "$typed" == *'typed-42\r\n'* && "$typed" != *'\x00'* ]] \
  || fail "a command typed before the shell started: the terminal showed $typed"
ok "a command typed before the shell starts runs, with no stray bytes on the terminal"
waited=0
until [[ "$(shells)" == "$shells_before" ]]; do
  ((waited < 30)) || fail "$(shells) bash processes 30s after the connection dropped, $shells_before before"
  sleep 1
  waited=$((waited + 1))
done
ok "dropping that connection ended its shell within ${waited}s"

as main.adam 'nohup node -e "require(\"http\").createServer((q, r) => r.end(\"forwarded\")).listen(8000, \"127.0.0.1\")" >/dev/null 2>&1 &'
ssh -F "$work/ssh_config" -o ExitOnForwardFailure=yes -N \
  -L "127.0.0.1:$forward_port:127.0.0.1:8000" main.adam@corral-e2e </dev/null &
tunnel=$!
body=
for _ in $(seq 50); do
  body=$(curl -s "http://127.0.0.1:$forward_port/") && break
  sleep 0.2
done
expect "ssh -L" forwarded "$body"
# Traffic through the forward is work, an open forward alone is not.
{ while curl -s -o /dev/null "http://127.0.0.1:$forward_port/"; do sleep 5; done; } &
held=$!
ok "ssh -L $forward_port:127.0.0.1:8000 reaches a server inside the sandbox"

if [[ -z "${CORRAL_E2E_SKIP_NETWORK_POLICY:-}" ]]; then
  expect "Corral's web port from the sandbox" 200 \
    "$(as main.adam "curl -s -m 5 -o /dev/null -w %{http_code} http://$release.$namespace.svc:8080/healthz")"
  code=$(as main.adam "curl -s -m 5 -o /dev/null http://$release-internal.$namespace.svc:8081/healthz; echo \$?")
  [[ "$code" != 0 ]] || fail "the sandbox reached Corral's internal port"
  ok "the NetworkPolicy keeps the sandbox off Corral's internal port (curl exit $code)"
fi

state() {
  api GET api/sandboxes/main | jq -r .state
}
sleep 45
expect "main's state with traffic through a forward" running "$(state)"
ok "traffic through a forward every 5s keeps main awake 45s into a 30s idle timeout"

stop "$held"
stop "$tunnel"
held=
tunnel=
waited=0
until [[ "$(state)" == sleeping ]]; do
  ((waited < 120)) || fail "main was still $(state) after ${waited}s"
  sleep 5
  waited=$((waited + 5))
done
kubectl --namespace "$sandboxes" wait --for=delete "pod/$sandbox_id" --timeout=60s >>"$log"
ok "with the forward idle and then closed, main slept within ${waited}s"

woke_at=$SECONDS
expect "the home after a wake" kept "$(as main.adam 'cat ~/marker')"
ok "ssh woke main in $((SECONDS - woke_at))s, home kept"

if [[ "$part" == all ]]; then
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
  expect "the host key Secret after an upgrade" "$host_key" "$(secret_data "$release-ssh-host-key")"
  login
  expect "keys after the upgrade" 1 "$(api GET api/keys | jq length)"
  expect "the home after the upgrade" kept "$(as main.adam 'cat ~/marker')"
  ok "helm upgrade restarted Corral with new config; the gate kept an open SSH session, and data, generated keys and host key survived"

  stop_forwards
  helm uninstall "$release" --namespace "$namespace" --wait >>"$log"
  expect "kept in $namespace" "persistentvolumeclaim/$release-data secret/$release-keys secret/$release-ssh-host-key" \
    "$(kubectl --namespace "$namespace" get pvc,secrets -o name | sort | xargs)"
  expect "kept in $sandboxes" "sandbox.agents.x-k8s.io/$sandbox_id" \
    "$(kubectl --namespace "$sandboxes" get sandboxes -o name)"
  ok "helm uninstall with keepOnUninstall: the database volume, keys, host key and sandboxes stay"

  install
  connect
  expect "the keys Secret after a reinstall" "$keys" "$(secret_data "$release-keys")"
  expect "the host key Secret after a reinstall" "$host_key" "$(secret_data "$release-ssh-host-key")"
  login
  expect "sandboxes after a reinstall" main "$(api GET api/sandboxes | jq -r '.[].name')"
  expect "the home after a reinstall" kept "$(as main.adam 'cat ~/marker')"
  ok "helm install over the kept data: same admin, same sandbox and home, same host key"

  install --set keepOnUninstall=false
  stop_forwards
  helm uninstall "$release" --namespace "$namespace" --wait >>"$log"
  kubectl wait --for=delete "namespace/$sandboxes" --timeout=180s >>"$log" 2>&1
  expect "left in $namespace" "secret/$release-keys secret/$release-ssh-host-key" \
    "$(kubectl --namespace "$namespace" get pvc,secrets -o name | sort | xargs)"
  ok "helm uninstall without keepOnUninstall deletes the database volume and the sandbox namespace; the generated keys stay"
fi

echo "chart e2e $part: all $passed checks passed"
