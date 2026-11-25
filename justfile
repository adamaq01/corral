# Corral's tests, in tiers: docs/testing.md. CI calls the ci-* and nightly-* recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Lists the recipes.
default:
    @just --list --unsorted

# T0, seconds: cargo check and the unit tests of the crates named, or of those changed against main, and svelte-check when web/ changed.
fast *crates:
    #!/usr/bin/env bash
    set -euo pipefail
    crates=({{ crates }})
    if ((${#crates[@]} == 0)); then
      mapfile -t crates < <(tests/affected.sh crates)
      if tests/affected.sh files | grep -q '^web/'; then
        just web-deps
        (cd web && npm run check)
      fi
    fi
    if ((${#crates[@]} > 0)); then
      packages=("${crates[@]/#/--package=}")
      cargo check "${packages[@]}" --all-targets --all-features
      cargo nextest run "${packages[@]}" --all-features --profile hermetic --no-tests=pass \
        -E 'kind(lib) + kind(bin)'
    fi

# T1, before a commit, no Docker: rustfmt, clippy, the hermetic tests, and the web and chart checks when web/ or deploy/helm/ changed.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo nextest run --workspace --all-features --profile hermetic
    if tests/affected.sh files | grep -q '^web/'; then just web-check; fi
    if tests/affected.sh files | grep -q '^deploy/helm/'; then just chart; fi

# T2 rust: every Rust test but the kind cluster's and HashiCorp Vault's, with Docker, on this checkout's sandbox and overlay images.
test-rust profile="default":
    tests/images.sh sandbox overlay
    cargo nextest run --workspace --all-features --profile {{ profile }}

# T2 web: prettier, svelte-check, then Playwright, which builds the UI and corral-server: the functional tests and two views per page.
test-web: web-check
    cd web && npx playwright install chromium
    cd web && npx playwright test --project chromium --project sandboxes

# T2 images: the sandbox image's tools, then the overlay on each base image and on the sandbox image.
images:
    images/sandbox/test.sh
    images/overlay/test.sh

# T2 compose: the Docker Compose install end to end; part=all adds Herdr, the installer on a terminal, idle sleep and the long checks.
e2e-compose part="smoke":
    tests/e2e/compose.sh {{ part }}

# T2 kind: on one kind cluster, the engine's cluster tests then the chart e2e, each run and reported; part=all adds the chart's lifecycle.
e2e-kind part="smoke":
    #!/usr/bin/env bash
    set -uo pipefail
    crates/corral-engine-agent-sandbox/tests/kind/up.sh || exit 1
    KUBECONFIG=$(tests/checkout.sh CORRAL_KUBECONFIG) CORRAL_TEST_KUBE_NAMESPACE=corral-dev \
      cargo nextest run --package corral-engine-agent-sandbox --all-features --profile kind
    engine=$?
    deploy/helm/e2e.sh {{ part }}
    chart=$?
    echo "kind: the engine's cluster tests exited $engine, the chart e2e $chart"
    ((engine == 0 && chart == 0))

# The chart without a cluster: lint, schemas, unit tests and its config against corral-server. Needs helm, kubeconform and helm-unittest (deploy/helm/tools.sh).
chart:
    cargo build --package corral-server
    deploy/helm/test.sh

# RustSec advisories against Cargo.lock. Needs cargo-deny.
deny:
    cargo deny check advisories

# T2 locally: the recipes for the jobs the changes against main reach, as CI picks its jobs (tests/affected.sh).
affected:
    #!/usr/bin/env bash
    set -uo pipefail
    jobs=$(tests/affected.sh jobs)
    echo "affected: ${jobs:-nothing}"
    failed=()
    for job in $jobs; do
      case $job in
        rust) recipe=test-rust ;;
        web) recipe=test-web ;;
        compose) recipe=e2e-compose ;;
        kind) recipe=e2e-kind ;;
        *) recipe=$job ;;
      esac
      just "$recipe" || failed+=("$recipe")
    done
    if ((${#failed[@]} > 0)); then
      echo "failed: ${failed[*]}" >&2
      exit 1
    fi

# T3, nightly and on release tags: what per-push CI leaves out, each run and reported.
nightly:
    #!/usr/bin/env bash
    set -uo pipefail
    failed=()
    for recipe in nightly-vault nightly-web nightly-compose nightly-kind nightly-opkssh; do
      just "$recipe" || failed+=("$recipe")
    done
    if ((${#failed[@]} > 0)); then
      echo "failed: ${failed[*]}" >&2
      exit 1
    fi

# CI rust job: rustfmt, clippy and test-rust with one retry, flaky tests reported, JUnit in target/nextest/ci/junit.xml.
ci-rust:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    just test-rust ci

# CI web job.
ci-web: test-web

# CI images job.
ci-images: images

# CI compose job: the smoke part.
ci-compose: (e2e-compose "smoke")

# CI kind job: the engine's cluster tests and the chart e2e without its lifecycle.
ci-kind: (e2e-kind "smoke")

# CI chart job.
ci-chart: chart

# Nightly: the secret store against HashiCorp Vault.
nightly-vault:
    cargo nextest run --package corral-secrets-vault --all-features --profile vault

# Nightly: the two views per page per-push leaves out, dark at desktop and light at phone width.
nightly-web: web-deps
    cd web && npx playwright install chromium
    cd web && npx playwright test --project other-views

# Nightly: the whole compose e2e.
nightly-compose: (e2e-compose "all")

# Nightly: the kind job with the chart's lifecycle: upgrade, uninstall and reinstall.
nightly-kind: (e2e-kind "all")

# Nightly: opkssh logins with VoidAuth as the issuer.
nightly-opkssh: web-deps
    cd web && npx playwright install chromium
    tests/e2e/opkssh.sh

# Removes this checkout's test images, and the kind cluster, compose stacks, containers, volumes and networks of its run (CORRAL_RUN_ID).
prune:
    tests/prune.sh

# Installs web/'s dependencies when package-lock.json is newer than node_modules.
[private]
web-deps:
    cd web && if [[ ! node_modules/.package-lock.json -nt package-lock.json ]]; then npm ci; fi

# Prettier and svelte-check.
[private]
web-check: web-deps
    cd web && npm run lint
    cd web && npm run check
