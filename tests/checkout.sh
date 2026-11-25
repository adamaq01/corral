#!/usr/bin/env bash
# Names what this checkout's tests make on a Docker host that other checkouts share, so two
# worktrees never use or remove each other's: the images' tags derive from the checkout's path,
# and the e2e stacks, kind cluster, container names and ports from the run id, which defaults to
# the same hash. A value already in the environment is kept.
#
# Sourced, it exports CORRAL_RUN_ID, CORRAL_SERVER_IMAGE, CORRAL_SANDBOX_IMAGE,
# CORRAL_OVERLAY_IMAGE and CORRAL_KUBECONFIG, and defines corral_port and corral_default_image.
# Run, it prints one of those variables: tests/checkout.sh CORRAL_SANDBOX_IMAGE.

corral_checkout=$(printf %s "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)" | sha256sum | cut -c1-8)

# The tag `tests/images.sh` builds for images/<name> when no prebuilt image is named.
corral_default_image() {
  echo "corral-$1:wt-$corral_checkout"
}

export CORRAL_RUN_ID=${CORRAL_RUN_ID:-$corral_checkout}
if [[ ! "$CORRAL_RUN_ID" =~ ^[a-z0-9]{1,20}$ ]]; then
  echo "CORRAL_RUN_ID must be 1 to 20 lowercase letters and digits: [$CORRAL_RUN_ID]" >&2
  return 1 2>/dev/null || exit 1
fi
export CORRAL_SERVER_IMAGE=${CORRAL_SERVER_IMAGE:-$(corral_default_image server)}
export CORRAL_SANDBOX_IMAGE=${CORRAL_SANDBOX_IMAGE:-$(corral_default_image sandbox)}
export CORRAL_OVERLAY_IMAGE=${CORRAL_OVERLAY_IMAGE:-$(corral_default_image overlay)}
# The kind cluster's own kubeconfig: a run never reads or switches the person's current context.
export CORRAL_KUBECONFIG=${CORRAL_KUBECONFIG:-${TMPDIR:-/tmp}/corral-kind-$CORRAL_RUN_ID.kubeconfig}

# Prints host port <n> (0 to 15) of this run's block of sixteen, which the run id picks, failing
# when something already listens there.
corral_port() {
  local block port
  block=$(($(printf %s "$CORRAL_RUN_ID" | cksum | cut -d' ' -f1) % 1000))
  port=$((10000 + block * 16 + $1))
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    echo "port $port of run $CORRAL_RUN_ID is taken: run with another CORRAL_RUN_ID" >&2
    return 1
  fi
  echo "$port"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  case "${1:-}" in
    CORRAL_RUN_ID | CORRAL_SERVER_IMAGE | CORRAL_SANDBOX_IMAGE | CORRAL_OVERLAY_IMAGE | CORRAL_KUBECONFIG)
      echo "${!1}"
      ;;
    *)
      echo "usage: tests/checkout.sh CORRAL_RUN_ID|CORRAL_{SERVER,SANDBOX,OVERLAY}_IMAGE|CORRAL_KUBECONFIG" >&2
      exit 2
      ;;
  esac
fi
