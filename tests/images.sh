#!/usr/bin/env bash
# Makes sure the test images are here: tests/images.sh [server] [sandbox] [overlay], all three by
# default. An image whose CORRAL_<NAME>_IMAGE is this checkout's own tag (tests/checkout.sh) is
# built from images/<name>; any other is prebuilt and must already be here.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo/tests/checkout.sh"

names=("$@")
if ((${#names[@]} == 0)); then
  names=(server sandbox overlay)
fi
for name in "${names[@]}"; do
  variable=CORRAL_${name^^}_IMAGE
  image=${!variable:?"unknown image $name: server, sandbox or overlay"}
  if [[ "$image" == "$(corral_default_image "$name")" ]]; then
    docker build --quiet --file "$repo/images/$name/Dockerfile" --tag "$image" "$repo" >/dev/null
    echo "built $image"
  elif docker image inspect "$image" >/dev/null 2>&1; then
    echo "prebuilt $image"
  else
    echo "no image $image here, which $variable names" >&2
    exit 1
  fi
done
