#!/usr/bin/env bash
# Removes this checkout's test images, and the kind cluster, compose stacks, containers, volumes
# and networks of its run, CORRAL_RUN_ID (tests/checkout.sh). Nothing of another run is touched.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo/tests/checkout.sh"

if kind get clusters 2>/dev/null | grep -qx "corral-$CORRAL_RUN_ID"; then
  "$repo/crates/corral-engine-agent-sandbox/tests/kind/down.sh"
fi

named="^(corral-e2e|corral-opkssh-e2e|corral-overlay-test)-$CORRAL_RUN_ID([-_]|$)"

# Sandboxes are named after their id, so they are found by the run's networks; their homes go too.
remove_container() {
  local volumes
  volumes=$(docker inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' "$1")
  docker rm --force "$1" >/dev/null
  for volume in $volumes; do
    docker volume rm "$volume" >/dev/null
  done
}

networks=$(docker network ls --format '{{.Name}}' | grep -E "$named" || true)
for network in $networks; do
  for container in $(docker ps --all --quiet --filter "network=$network"); do
    remove_container "$container"
  done
done
for container in $(docker ps --all --format '{{.Names}}' | grep -E "$named" || true); do
  remove_container "$container"
done
for network in $networks; do
  docker network rm "$network" >/dev/null
done
for volume in $(docker volume ls --format '{{.Name}}' | grep -E "$named" || true); do
  docker volume rm "$volume" >/dev/null
done
images=$(docker image ls --format '{{.Repository}}:{{.Tag}}' |
  grep -E "^corral-(server|sandbox|overlay):wt-$corral_checkout$|^corral-overlay-test-$CORRAL_RUN_ID-" || true)
for image in $images; do
  docker image rm "$image" >/dev/null
done
echo "pruned run $CORRAL_RUN_ID and the images of checkout $corral_checkout"
