#!/usr/bin/env bash
# What the changes since the merge base with main (CORRAL_BASE, main by default) touch, committed
# or not: tests/affected.sh jobs|crates|files, or all for every job. The jobs are the per-push CI
# jobs, which CI picks with this script; `just affected` runs the matching recipes.
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

all_jobs=(rust deny web images compose kind chart)

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
  for job in "${all_jobs[@]}"; do
    if [[ -n "${wanted[$job]:-}" ]]; then
      echo "$job"
    fi
  done
}

case "${1:-}" in
  files) files ;;
  jobs) jobs ;;
  all) printf '%s\n' "${all_jobs[@]}" ;;
  crates) files | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u ;;
  *)
    echo "usage: tests/affected.sh jobs|all|crates|files" >&2
    exit 2
    ;;
esac
