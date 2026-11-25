# Testing

Every test runs through one entry point, the `justfile` at the repository root. `just` lists the
recipes. CI runs the same recipes: the `ci-*` ones on each push, the `nightly-*` ones nightly and on
release tags.

## Tiers

| Tier | When | Recipe | What it runs |
| --- | --- | --- | --- |
| T0 | on save, seconds | `just fast [crate...]` | `cargo check` and the unit tests of the crates named, or of those changed against `main`; `svelte-check` when `web/` changed |
| T1 | before a commit, no Docker | `just check` | rustfmt, clippy, the hermetic tests, prettier and `svelte-check` when `web/` changed, `deploy/helm/test.sh` when `deploy/helm/` changed |
| T2 | each push | `just affected` | the recipes below that the changes against `main` reach |
| T3 | nightly | `just nightly` | what T2 leaves out, every part run and reported |

The T2 recipes, one per CI job:

- `just test-rust`: every Rust test with Docker, on this checkout's sandbox and overlay images,
  except the kind cluster tests and HashiCorp Vault. Postgres and OpenBao run in containers.
- `just test-web`: prettier, `svelte-check`, and Playwright, which builds the UI and
  corral-server once: the functional tests, and each page's look and accessibility in two views,
  light at desktop width and dark at phone width.
- `just images`: the sandbox image's tools, and the overlay on each base image.
- `just e2e-compose`: the smoke part of the Docker Compose install, `tests/e2e/compose.sh`: the
  stack comes up, sign-in, SSH, refusals, the environment, `corral-new`, persistence across
  restarts, deletion.
- `just e2e-kind`: on one kind cluster, the agent-sandbox engine's cluster tests, then the chart
  e2e, `deploy/helm/e2e.sh`; the second runs when the first fails, and both are reported.
- `just chart`: the chart without a cluster, `deploy/helm/test.sh`.
- `just deny`: RustSec advisories.

T3 adds `nightly-vault` (the secret store against HashiCorp Vault), `nightly-web` (the other two
views of each page, dark at desktop and light at phone width), `nightly-compose` (the whole compose
e2e: Herdr, the installer on a terminal, the `corral` command inside a sandbox, a slow wake, idle
sleep and keep-awake), `nightly-kind` (the chart's upgrade, uninstall and reinstall) and
`nightly-opkssh` (opkssh logins with VoidAuth as the issuer).

`tests/affected.sh jobs` prints the T2 jobs the changes reach; the table at its top maps paths to
jobs. Markdown and `docs/` reach none, `crates/` and `Cargo.lock` reach every job downstream.

## nextest profiles

`.config/nextest.toml` holds the profiles `cargo nextest run --profile <name>` takes:

- `default`: every test but the kind cluster tests, the compose stack test and HashiCorp Vault.
  A test that needs the sandbox or overlay image passes after printing why it skipped when the
  image is not here, and one of them shows that line at the end of every run. With `CI` set, it
  fails instead. `just test-rust` builds the images first, so nothing skips there.
- `hermetic`: no Docker at all, for `just check`.
- `ci`: `default` with one retry, flaky tests reported, and JUnit in
  `target/nextest/ci/junit.xml`, under the repository whatever `CARGO_TARGET_DIR` is.
- `kind`: the agent-sandbox cluster tests, one at a time.
- `vault`: HashiCorp Vault.

## Prerequisites

- Rust from `rust-toolchain.toml`, [just](https://just.systems) and
  [cargo-nextest](https://nexte.st). Without root, put their release binaries in `~/.cargo/bin`.
- Docker for T2 and T3. Node 22 for `web/`, with Playwright's Chromium, which `test-web` installs.
- `chart`: helm, kubeconform and helm-unittest, which `deploy/helm/tools.sh <dir>` installs.
- `e2e-kind`: kind, kubectl and helm. `nightly-compose`: Herdr's `hc`. `nightly-opkssh`: openssl.
- `deny`: cargo-deny.

The recipes honor `CARGO_TARGET_DIR`.

## Worktrees and runs

Several checkouts can run their tests on one Docker host at once. `tests/checkout.sh` names what
they make:

- Images: `corral-server`, `corral-sandbox` and `corral-overlay`, tagged `wt-<hash of the
  checkout's path>`. Each script and test that needs one builds it from `images/` (Docker's cache
  makes that quick when nothing changed); the Rust tests, which cannot, skip without it outside
  CI. `CORRAL_SERVER_IMAGE`, `CORRAL_SANDBOX_IMAGE` and `CORRAL_OVERLAY_IMAGE` name prebuilt
  images instead, which must be here.
- Everything else, from the run id `CORRAL_RUN_ID` (letters and digits, the same hash by
  default): the compose projects `corral-e2e-<run>` and `corral-opkssh-e2e-<run>`, their networks
  and volumes, the overlay test's containers, the kind cluster `corral-<run>` with its kubeconfig
  in `$TMPDIR`, and a block of host ports. Two runs from one checkout need different run ids.

Each script removes only its own run's resources. `just prune` removes this checkout's images and
everything of its run, the kind cluster included; `CORRAL_RUN_ID=<run> just prune` removes another
run's.
