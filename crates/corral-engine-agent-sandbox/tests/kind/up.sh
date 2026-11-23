#!/usr/bin/env bash
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
cluster=corral-dev
context=kind-$cluster
node=$cluster-control-plane
namespace=corral-dev
agent_sandbox_version=v1.0.11
sandbox_image=${CORRAL_SANDBOX_IMAGE:-corral-sandbox:dev}
overlay_image=${CORRAL_OVERLAY_IMAGE:-corral-overlay:dev}
agent_image=${CORRAL_AGENT_IMAGE:-containerssh/agent:v0.11.0}
proxy_ca=/usr/local/share/ca-certificates/proxy-ca.crt

if ! kind get clusters | grep -x "$cluster" >/dev/null; then
  kind create cluster --name "$cluster" --config "$here/kind.yaml" --wait 120s
fi
kubectl config use-context "$context" >/dev/null

# A TLS-intercepting proxy needs its CA inside the node, or image pulls fail with x509 errors.
if [[ -f "$proxy_ca" ]] && ! docker exec "$node" test -f "/usr/local/share/ca-certificates/$(basename "$proxy_ca")"; then
  docker cp "$proxy_ca" "$node:/usr/local/share/ca-certificates/"
  docker exec "$node" sh -c 'update-ca-certificates && systemctl restart containerd'
fi

kubectl --context "$context" apply --server-side \
  -f "https://github.com/kubernetes-sigs/agent-sandbox/releases/download/$agent_sandbox_version/sandbox.yaml"
kubectl --context "$context" -n agent-sandbox-system rollout status deployment/agent-sandbox-controller --timeout=180s

kubectl --context "$context" apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $namespace
  labels:
    pod-security.kubernetes.io/enforce: baseline
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: corral-sandbox
  namespace: $namespace
automountServiceAccountToken: false
EOF

# Loads a local image into the node unless it holds that image already, building it first from
# images/<directory> when Docker does not have it.
load() {
  if ! docker image inspect "$1" >/dev/null 2>&1; then
    docker build --file "$repo/images/$2/Dockerfile" --tag "$1" "$repo"
  fi
  local id
  id=$(docker image inspect --format '{{.Id}}' "$1")
  if ! docker exec "$node" ctr --namespace=k8s.io images ls | grep -F "$id" >/dev/null; then
    if ! kind load docker-image "$1" --name "$cluster"; then
      docker save "$1" | docker exec -i "$node" ctr --namespace=k8s.io images import -
    fi
  fi
}
load "$sandbox_image" sandbox
load "$overlay_image" overlay
docker exec "$node" crictl pull "$agent_image" >/dev/null
# The volume provisioner's helper image ships packed, so the prune below would drop its layers.
helper_image=$(kubectl --context "$context" -n local-path-storage get configmap local-path-config \
  -o jsonpath='{.data.helperPod\.yaml}' | awk '$1 == "image:" { print $2 }')
docker exec "$node" crictl pull "$helper_image" >/dev/null
# The node only needs the unpacked snapshots; dropping the layer blobs saves about 1 GB of Docker disk.
docker exec "$node" ctr --namespace=k8s.io content prune references

echo "kind cluster $cluster is ready: namespace $namespace, images $sandbox_image, $overlay_image and $agent_image"
