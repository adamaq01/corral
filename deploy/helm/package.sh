#!/usr/bin/env bash
# Packages the chart into the directory given. On a tag it is versioned as the tag; otherwise as
# <Chart.yaml version>-main.<CI run number>, so newer builds sort higher, running the image CI
# tagged sha-<7-character commit> for the same commit.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=${1:?usage: package.sh <directory>}
ref_type=${GITHUB_REF_TYPE:-branch}
ref_name=${GITHUB_REF_NAME:-}
sha=${GITHUB_SHA:-$(git -C "$here" rev-parse HEAD)}

if [[ "$ref_type" == tag ]]; then
  app_version=$ref_name
  version=${ref_name#v}
else
  app_version=sha-${sha:0:7}
  version=$(sed -n 's/^version: //p' "$here/corral/Chart.yaml")-main.${GITHUB_RUN_NUMBER:-0}
fi
mkdir -p "$out"
helm package "$here/corral" --version "$version" --app-version "$app_version" --destination "$out"
