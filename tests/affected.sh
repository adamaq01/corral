#!/usr/bin/env bash
# What the changes since the merge base with main (CORRAL_BASE, main by default) touch, committed
# or not: tests/affected.sh jobs|crates|files. The jobs are the per-push CI jobs; `just affected`
# runs the matching recipes, and CI's path filters mirror the table below.
set -euo pipefail

# The first pattern a changed path matches decides its jobs; a `*` crosses directories.
rules() {
  cat <<'EOF'
*.md                                          -
docs/*                                        -
tests/e2e/opkssh*                             -
deny.toml                                     deny
Cargo.lock                                    rust deny web images compose kind chart
crates/corral-engine-agent-sandbox/tests/kind/*  kind
.config/nextest.toml                          rust kind
deploy/compose/*                              compose
tests/e2e/compose.sh                          compose
deploy/helm/e2e.sh                            kind
deploy/helm/*                                 chart kind
web/*                                         web compose
images/*                                      rust web images compose kind
crates/*                                      rust web images compose kind chart
*                                             rust web images compose kind chart deny
EOF
}

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo"

base=${CORRAL_BASE:-main}
if ! git rev-parse --verify --quiet "$base" >/dev/null; then
  base=origin/$base
fi

files() {
  {
    git diff --name-only "$(git merge-base HEAD "$base")"
    git ls-files --others --exclude-standard
  } | sort -u
}

jobs() {
  local path pattern selected
  declare -A wanted=()
  while read -r path; do
    while read -r pattern selected; do
      if [[ "$path" == $pattern ]]; then
        for job in $selected; do
          [[ "$job" == - ]] || wanted[$job]=1
        done
        break
      fi
    done < <(rules)
  done < <(files)
  for job in rust deny web images compose kind chart; do
    if [[ -n "${wanted[$job]:-}" ]]; then
      echo "$job"
    fi
  done
}

case "${1:-}" in
  files) files ;;
  jobs) jobs ;;
  crates) files | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u ;;
  *)
    echo "usage: tests/affected.sh jobs|crates|files" >&2
    exit 2
    ;;
esac
