# Corral web UI

A SvelteKit single-page app with shadcn-svelte components. `npm run build` writes it to `build/`,
which corral-server serves from `[http] ui_dir` (the server image puts it in
`/usr/share/corral/web`), answering every path outside `/api` and `/auth` with `index.html`.

## Develop

Run a corral-server on `http://127.0.0.1:8080` (or set `CORRAL_URL`), then:

```sh
npm ci
npm run dev
```

The dev server proxies `/api` and `/auth` to that corral-server, so the browser sees one origin and
the session cookie just works. Sign in with a local account made with
`corral-server local-account create <name>`. A redirect provider such as OIDC returns to the
server's `base_url`, so it only works in dev when `base_url` is the dev server's address.

## Check

```sh
npm run lint      # prettier
npm run check     # svelte-check
npm run build
npm run test:e2e  # Playwright
```

The Playwright suite builds the UI and the corral-server binary (set `CORRAL_SERVER_BIN` to use a
built one), then runs the server on SQLite in a temporary directory with local accounts and the
Docker engine on a network of its own, and `corral-server gate` beside it as the SSH front door,
so tests SSH into the sandboxes they create. The suite removes the containers, volumes and network
afterwards.
Sandboxes use this checkout's sandbox and overlay images, which `tests/images.sh` builds first,
unless `CORRAL_SANDBOX_IMAGE` and `CORRAL_OVERLAY_IMAGE` name other images (see
[docs/testing.md](../docs/testing.md)). A failed test keeps its trace in `test-results/`, next to
`corral-server.log` and `corral-gate.log`.

The `*screenshots.spec.ts` files open every page in both themes at desktop and phone width, full,
empty, loading, failed and with its dialogs open, with fixed data in place of the API's. The
`chromium` project checks light at desktop width and dark at phone width, and `other-views` the
other two; `npx playwright test` runs every project. Each view must pass axe with no serious or critical violation, have one `h1` and headings that skip no
level, never scroll sideways, and show focus on every element Tab reaches. With
`CORRAL_SCREENSHOTS=<dir>`, they also save a screenshot of each view there.

## API types

`src/lib/api/generated/` is generated from the Rust API types; never edit it. After changing an
API type, regenerate it from the repository root:

```sh
CORRAL_WRITE_TYPES=1 cargo test -p corral-server web_types
```

## Components

`src/lib/components/ui/` holds shadcn-svelte components as its CLI writes them. Add one with
`npx shadcn-svelte@latest add <component>`.
