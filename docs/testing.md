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

## CI

`.github/workflows/ci.yml` runs on each push to `main` and `ci/**`, on pull requests, on tags, every
night at 03:17 UTC on `main`, and by hand (`workflow_dispatch`).

Its `changes` job runs `tests/affected.sh jobs` over the changes since the last commit whose push
run passed on the branch, or since a pull request's base, so a cancelled or failed run's changes
are picked again by the next push. A branch with no passing run, a tag, the nightly run and a run
by hand take every job. A push that changes only docs runs nothing else.

When a job that needs images is picked, the images are built once, natively for amd64 and arm64,
and pushed as `ghcr.io/adamaq01/<image>:ci-<commit>`; each job pulls them under the names it tests.
Each job runs one recipe:

| Job | Recipe | Needs the images |
| --- | --- | --- |
| `rust` | `just ci-rust`: rustfmt, clippy, `test-rust` with the `ci` profile | yes |
| `web` | `just ci-web` | yes |
| `images-test` | `just ci-images` | yes |
| `compose` | `just ci-compose` | yes |
| `kind` | `just ci-kind` | yes |
| `chart` | `just ci-chart` | no |
| `deny` | `cargo deny check advisories`, as `just deny` | no |

The nightly run, tags and a run by hand add `nightly-vault` to `rust` and `nightly-web` to `web`,
run `nightly-compose` and `nightly-kind` in place of the smoke parts, and run `nightly-opkssh` in
its own job. A failed nightly run opens the issue "The nightly CI run fails", or comments on it;
the next green night closes it.

On a push to `main` or a tag, once no job failed or was cancelled, `publish` retags the run's
images `sha-<commit>`, `latest` on `main` and the tag's name on a tag, and pushes the chart with
that image tag as its appVersion. A run that built no images publishes nothing; a chart change
reaches the `kind` job, so it builds images and publishes both.

The nightly run also deletes `ci-*` images and untagged versions older than three days from the
three image packages and `corral-buildcache`, keeping any version with another tag and the
platform images of every kept tag. A run by hand off `main` only lists what it would delete.

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
