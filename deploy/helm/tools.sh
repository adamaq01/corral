#!/usr/bin/env bash
# Installs the pinned helm, kubeconform and helm-unittest into the directory given, each checked
# against its published SHA-256. Linux on amd64 or arm64.
set -euo pipefail

bin=${1:?usage: tools.sh <directory>}
mkdir -p "$bin"
bin=$(cd "$bin" && pwd)
arch=$(uname -m)
case "$arch" in
  x86_64) arch=amd64 ;;
  aarch64 | arm64) arch=arm64 ;;
  *) echo "unsupported architecture $arch" >&2; exit 1 ;;
esac

helm_version=v4.3.2
kubeconform_version=v0.8.0
unittest_version=1.2.1
declare -A sha256=(
  [helm-amd64]=d1c0278d46beab79582d7e50f5b3dc206944ba8e577cde618513fd07daa53983
  [helm-arm64]=df31cb61680ef728f82c770e5280dd565617bee6b039d37e85c677288be4bbb0
  [kubeconform-amd64]=9bc2bffbf71f261128533edaf912153948b7ff238f9a531ae6d34466ec287883
  [kubeconform-arm64]=1f53fc8e81258197a35e8603054162a5af1de8c5af13746c71ab680d9534ed87
  [unittest-amd64]=48e86ecc1467b009630e5954375e5d8494139f73b466df5fe8c04fa45b634939
  [unittest-arm64]=c66062713cd3cbb1a80d7098bd1d4a48db8438f56b43fb6377df65447f93f8de
)

download=$(mktemp -d)
trap 'rm -rf "$download"' EXIT

fetch() {
  local url=$1 sum=$2 archive
  archive=$download/$(basename "$url")
  curl -fsSL -o "$archive" "$url"
  echo "$sum  $archive" | sha256sum --check --quiet -
  mkdir -p "$download/unpacked"
  tar -xzf "$archive" -C "$download/unpacked"
}

fetch "https://get.helm.sh/helm-$helm_version-linux-$arch.tar.gz" "${sha256[helm-$arch]}"
install -m 755 "$download/unpacked/linux-$arch/helm" "$bin/helm"
fetch "https://github.com/yannh/kubeconform/releases/download/$kubeconform_version/kubeconform-linux-$arch.tar.gz" \
  "${sha256[kubeconform-$arch]}"
install -m 755 "$download/unpacked/kubeconform" "$bin/kubeconform"
fetch "https://github.com/helm-unittest/helm-unittest/releases/download/v$unittest_version/helm-unittest-linux-$arch-$unittest_version.tgz" \
  "${sha256[unittest-$arch]}"
install -m 755 "$download/unpacked/untt-linux-$arch" "$bin/helm-unittest"

"$bin/helm" version --short
"$bin/kubeconform" -v
"$bin/helm-unittest" --help >/dev/null && echo "helm-unittest $unittest_version"
