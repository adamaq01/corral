#!/usr/bin/env bash
# Checks the chart without a cluster: lint, the values schema, Kubernetes schemas of the rendered
# manifests, the rendered config against corral-server, and the unit tests in corral/tests.
# Needs helm, kubeconform and helm-unittest on PATH (tools.sh installs them), and CORRAL_SERVER,
# a corral-server binary (target/debug/corral-server by default).
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
chart=$here/corral
server=${CORRAL_SERVER:-$repo/target/debug/corral-server}
# CRD schemas for the HTTPRoute and the CloudNativePG Cluster the chart can render.
crds=https://raw.githubusercontent.com/datreeio/CRDs-catalog/04af58d740718c4dc719441bf39fb284c784f4a8
kube_version=1.37.0

failures=0
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pass() {
  echo "ok - $*"
}

failed() {
  echo "FAIL - $*" >&2
  failures=$((failures + 1))
}

for tool in helm kubeconform helm-unittest; do
  command -v "$tool" >/dev/null || { echo "$tool is not on PATH: run deploy/helm/tools.sh" >&2; exit 1; }
done
[[ -x "$server" ]] || { echo "no corral-server at $server: cargo build -p corral-server" >&2; exit 1; }

render() {
  helm template corral "$chart" --namespace corral --kube-version "$kube_version" \
    --api-versions agents.x-k8s.io/v1beta1 "$@"
}

for example in "$here"/examples/*.yaml; do
  name=$(basename "$example")
  if output=$(helm lint --strict "$chart" --values "$example" 2>&1); then
    pass "helm lint --strict with $name"
  else
    failed "helm lint --strict with $name: $output"
  fi
  if ! render --values "$example" >"$work/$name" 2>"$work/$name.err"; then
    failed "$name does not render: $(cat "$work/$name.err")"
    continue
  fi
  if output=$(kubeconform -strict -summary -kubernetes-version "$kube_version" \
    -schema-location default \
    -schema-location "$crds/{{.Group}}/{{.ResourceKind}}_{{.ResourceAPIVersion}}.json" \
    "$work/$name" 2>&1); then
    pass "kubeconform with $name: ${output##*$'\n'}"
  else
    failed "kubeconform with $name: $output"
  fi
  render --values "$example" --show-only templates/configmap.yaml \
    | awk '/^  config.toml: \|/ { body = 1; next } body { sub(/^    /, ""); print }' >"$work/$name.toml"
  if output=$("$server" --config "$work/$name.toml" config check 2>&1); then
    pass "corral-server config check accepts the config rendered from $name"
  else
    failed "corral-server config check rejects the config rendered from $name: $output"
  fi
done

if output=$(helm template corral "$chart" 2>&1); then
  failed "the chart rendered without baseUrl and ssh.publicHost"
elif [[ "$output" != *"missing property 'baseUrl'"* || "$output" != *"at '/ssh': missing property 'publicHost'"* ]]; then
  failed "the chart's own defaults failed for another reason: $output"
else
  pass "the chart's own defaults are rejected until baseUrl and ssh.publicHost are set"
fi

awk -v dir="$work" '
  BEGIN { n = 1; file = dir "/rejected-1.yaml" }
  /^---$/ { n++; file = dir "/rejected-" n ".yaml"; next }
  { print > file }
' "$here/schema-tests/rejected.yaml"
for case in "$work"/rejected-*.yaml; do
  expect=$(sed -n 's/^# expect: //p' "$case")
  if output=$(render --values "$here/examples/minimal.yaml" --values "$case" 2>&1); then
    failed "accepted, expected [$expect]: $(grep -v '^#' "$case" | tr '\n' ' ')"
  elif [[ "$output" != *"$expect"* ]]; then
    failed "expected [$expect], got: $output"
  else
    pass "rejected: $expect"
  fi
done

if output=$(helm-unittest --strict "$chart" 2>&1); then
  pass "helm-unittest: $(grep -E '^Tests:' <<<"$output")"
else
  failed "helm-unittest: $output"
fi

if ((failures > 0)); then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "all chart checks passed"
