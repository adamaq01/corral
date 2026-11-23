#!/usr/bin/env bash
# Checks the image's tools, then that the Corral overlay makes it a whole sandbox, with the
# overlay's own test.
# CORRAL_SANDBOX_IMAGE names a prebuilt image; when unset, corral-sandbox:dev is built from here.
# CORRAL_OVERLAY_IMAGE names a prebuilt overlay, as for images/overlay/test.sh.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
image=${CORRAL_SANDBOX_IMAGE:-corral-sandbox:dev}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

if [[ -z "${CORRAL_SANDBOX_IMAGE:-}" ]]; then
  docker build --quiet --file "$here/Dockerfile" --tag "$image" "$repo" >/dev/null
  echo "built $image"
fi

for tool in claude codex python3 node git gcc; do
  out=$(docker run --rm "$image" "$tool" --version 2>&1) || fail "$tool --version failed: $out"
  [[ "$out" =~ [0-9]+\.[0-9]+\.[0-9]+ ]] || fail "$tool --version printed no version: $out"
  echo "ok - $tool --version: ${out%%$'\n'*}"
done

for piece in corral hc herdr-core sshd tini; do
  if docker run --rm "$image" sh -c "command -v $piece" >/dev/null; then
    fail "the image has $piece, which the overlay brings"
  fi
done
echo "ok - the image has none of the overlay's pieces"

CORRAL_OVERLAY_TEST_BASES=$image "$repo/images/overlay/test.sh"
