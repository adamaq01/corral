#!/usr/bin/env bash
# Packages the chart into the directory given, versioned like the Corral image CI publishes for
# the same commit: on a tag, the tag; otherwise <Chart.yaml version>-sha-<7-character commit>,
# running the image tagged sha-<7-character commit>.
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
  version=$(sed -n 's/^version: //p' "$here/corral/Chart.yaml")-$app_version
fi
mkdir -p "$out"
helm package "$here/corral" --version "$version" --app-version "$app_version" --destination "$out"
