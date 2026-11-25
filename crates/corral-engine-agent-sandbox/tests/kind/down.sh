#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)
source "$repo/tests/checkout.sh"
kind delete cluster --name "corral-$CORRAL_RUN_ID" --kubeconfig "$CORRAL_KUBECONFIG"
rm -f "$CORRAL_KUBECONFIG"
