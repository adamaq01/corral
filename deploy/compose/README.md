# Corral on Docker Compose

Two containers of the same image run the whole service: Corral (the web UI and API, local
accounts, SQLite and the Docker engine) and the gate (`corral-server gate`, the SSH front door),
which reaches Docker through a third, `docker-proxy`, that lets it run sessions and nothing else.
Each sandbox is a Docker container with its home on a named volume, from any image: the Corral
overlay, mounted into every sandbox, brings the sandbox user, sudo, OpenSSH, Herdr and the
`corral` command ([docs/images.md](../../docs/images.md)). Users reach a sandbox with any SSH
client:

```
ssh -p 48291 <sandbox>.<handle>@<public_host>
```

A bare `<handle>` reaches the user's default sandbox. A sandbox name the user has not used yet
makes that sandbox from their **Account** defaults, then opens it, unless they already have as
many sandboxes as their limit allows. A connection that reaches no sandbox, such as one past that
limit, to a sandbox scheduled for deletion, or by a disabled user, is told why at once: a command
prints the reason on its standard error and exits 1, a terminal shows it, and the connection ends
with it.

The gate waits for the first thing a connection asks (a shell, a command, sftp or a forward)
before it wakes or makes the sandbox; on a terminal it counts the seconds while the sandbox wakes.
Each session then runs through the overlay's own OpenSSH, started in the sandbox for that
connection, so it behaves exactly as OpenSSH: the login shell, variables, signals, exit status,
sftp, port and socket forwards both ways, and agent forwarding.

## Install

You need Docker Engine 28 or later, whose image mounts bring the overlay into sandboxes, with the
Compose plugin, and `ssh-keygen`.

1. Copy this directory to the host and keep it private, since it will hold the SSH host key and
   the key that decrypts every stored secret:

   ```sh
   cp -R deploy/compose /opt/corral && cd /opt/corral && chmod 700 .
   ```

2. In `config.toml`, set `base_url` (the address people open in a browser) and `public_host`
   (the host name people pass to `ssh`).

3. Generate Corral's keys: the session key, the key that encrypts stored secrets, and the
   secret Corral and the gate prove themselves to each other with. Run this once: new keys make
   every stored secret unreadable.

   ```sh
   docker compose run --rm corral keys generate > .env
   ```

4. Generate the gate's SSH host key:

   ```sh
   ssh-keygen -q -t ed25519 -N '' -C corral -f ssh_host_key
   ```

   `ssh-keygen -lf ssh_host_key.pub` prints the fingerprint users see on first connect.

5. Start the services and wait until they are healthy:

   ```sh
   docker compose up -d --wait
   ```

6. Create the first admin. The password is prompted for twice, at least 12 characters:

   ```sh
   docker compose exec corral corral-server local-account create <name> --admin
   ```

## First sandbox

Open `base_url` in a browser and sign in with the admin account from step 6. Then:

1. On **Account**, under **SSH keys**, add your public key, such as the contents of
   `~/.ssh/id_ed25519.pub`.
2. Under **Sandboxes**, create a sandbox. The first one is called `main` unless you name it
   otherwise, and it pulls the sandbox image (`ghcr.io/adamaq01/corral-sandbox`, Ubuntu with
   Node.js, Claude Code, Codex and Python, about 2 GB unpacked), so it can take a few minutes to
   start.
3. Copy a command from the sandbox's page and connect:

   ```sh
   ssh -p 48291 main.<name>@<public_host>
   ```

The sandbox's page also shows the Herdr command (`hc machine add ...`) and an `~/.ssh/config`
block, and wakes, stops, restarts or deletes the sandbox.

- `ssh -p 48291 <name>@<public_host>` opens the default sandbox: the first one created, or the
  one set with **Make default** on its page.
- Port forwarding works: `ssh -p 48291 -L 3000:127.0.0.1:3000 main.<name>@<public_host>`.
- Connecting as `<sandbox>.<name>` to a sandbox you do not have yet makes it: on a terminal, an
  installer asks for its image, size and environment, then opens its shell;
  `ssh ... <sandbox>.<name>@<public_host> corral-new` makes it from a JSON spec on stdin, for
  scripts; any other command makes it from your defaults. [docs/lobby.md](../../docs/lobby.md)
  has the installer and the spec.
- Admins set what Corral offers under **Settings**: the catalog of images and sizes and its
  defaults, the limits on sandboxes per member (5 by default) and sandboxes running at once (6),
  and registry logins. What `config.toml` sets under `[catalog]`, `[limits]` and `[[registries]]`
  shows there locked, and Settings takes more images, sizes and registries beside it. With no
  image in either, the catalog offers the sandbox image, and with no size, one of 2 CPUs and
  4 GiB. Settings marks those **Built in**, and the first image or size added there replaces
  them.
- Your **Account** sets the image and size a new sandbox gets unless you pick others, and the
  installer starts from them. Until you set them, they are the catalog's defaults.
- Users can also name an image of their own once the admin allows it under **Settings** >
  **Catalog**, with the registries it may come from and the most it may take to download.
  Registry logins go under **Settings** > **Registries**, each password in the `.env` variable
  the login names, and users never see them. [docs/images.md](../../docs/images.md) has both.
- A change to a catalog entry's `reference` or size reaches each sandbox made from it at its
  next start from sleep, its home kept; a running sandbox keeps what it has until then. A
  sandbox whose image or size is no longer in the catalog keeps running as it was made. Docker
  volumes have no size, so `home_size` does nothing here.
- **Environment** holds your variables (`NAME=value` in every SSH session, upper or lower case)
  and files (written into the home at every start, replacing what is at their path, missing
  directories made). Each name or path is yours once. Either can be secret: a secret value is
  encrypted with `CORRAL_SECRETS_KEY` and never shown again, only its length, first 4 and last 2
  characters once it has 16 or more (a file only its size), and a secret file is readable by the
  sandbox user alone. A file can be executable. A file marked as a template takes the value of
  each variable attached to the same sandbox in place of each `{{ NAME }}`, `NAME` an upper-case
  variable name; any other `{{` stays as written, so Go, Jinja or Helm files pass through. A
  variable takes at most 128 KiB, a file 256 KiB, and a sandbox's files and its variables at
  most 1 MiB each.
- An entry's name or path, kind (variable or file) and secrecy can change later; it keeps its
  sandboxes and their overrides. Corral refuses a change, a detach or a deletion that would keep
  a sandbox, or new sandboxes, from starting, such as removing a variable a template attached
  there refers to, and says why.
- A sandbox gets the variables and files attached to it: each new sandbox those marked
  **Apply to new sandboxes**, and on its page you attach or detach others, or override one with
  a value for that sandbox alone. Changes reach a sandbox from its next wake or restart on, and a
  running sandbox shows **Restart to apply** until then. A file Corral wrote that is no longer
  attached is removed at the next start, unless it was changed inside the sandbox; a file it
  could not write, or left for that reason, is reported at login and by `corral status`.
- Admins find everyone under **People**: they add local accounts and reset their passwords,
  disable, enable or delete users, and give a user a sandbox allowance in place of the limit per
  member. **All sandboxes** lists every user's sandboxes, and admins act on any of them there.
- Deleting a user, as an admin or from your own **Account**, deletes their sandboxes with their
  homes, keys, environment and local account. The username is free again afterwards, and a new
  account with it starts empty. A deletion that fails part way leaves the user with status
  Deleting, unable to log in, and deleting them again finishes it.
- A sandbox sleeps (its container stops) after 30 minutes with no work, and wakes on the next SSH
  login. Work is what a person does through SSH (typing, commands, file transfers, forwarded
  traffic) or the sandbox's own processor and network use, which it reports to Corral every
  minute: an agent answering or running tools is at work, one waiting at its prompt is not. An open
  but untouched connection does not keep a sandbox awake, and neither does Herdr: while the sandbox
  sleeps, the gate answers Herdr for it, so Herdr shows the machine connected with no terminals,
  and the first terminal opened there wakes it. Sleep keeps the container, so its files survive
  until the sandbox is deleted, and ends its programs.
- Each sandbox's page sets how long it stays awake with no work, up to `max_timeout_secs`, and
  keeps it awake for a while (an hour, four, until tomorrow morning) whatever it does. `[idle]` in
  `config.toml` sets the default timeout, those limits, and what counts as busy
  (`busy_cpu_percent` of one core, `busy_network_bytes_per_sec`). A sandbox that reports nothing,
  such as one started before an upgrade to this version, stays awake until it is restarted, and
  Corral logs that it does.

### From inside a sandbox

The `corral` command acts on the sandbox it runs in:

- `corral status` shows the sandbox and its state.
- `corral restart` and `corral stop` restart or stop it. Corral accepts the request and does it
  next, so the session that asked ends as the sandbox stops.
- `corral keep-awake 4h` keeps it awake for that long (`30m`, `4h`, `2d`), with or without work,
  and `corral keep-awake off` lets it sleep once idle again.
- `corral delete --confirm <sandbox>` stops the sandbox at once and deletes it, with its home, 10
  minutes later. It is refused until you turn it on in your **Account**. Until the deletion, the
  sandbox stays stopped and its page offers **Cancel deletion**.

At every start Corral writes `~/.config/corral/sandbox.json`, readable only by the sandbox user,
with a token that works for that sandbox alone while it runs. The command calls Corral's API at
`base_url`, so sandboxes must reach that address: with the default port mapping, the host's LAN
name or address does. Set `sandbox_base_url` under `[http]` when sandboxes reach Corral at another
address.

The web UI runs on a JSON API at the same address (`/api` and `/auth`), which scripts can call
too. Every `POST` and `PUT` there needs `Content-Type: application/json`, including the ones
without a body, such as `POST /api/sandboxes/main/stop`.

## Single sign-on and opkssh

To sign in with an OpenID Connect issuer such as VoidAuth instead of, or next to, local accounts,
register `<base_url>/auth/oidc/callback` as a confidential client's redirect URI, add the client's
secret to `.env` as `CORRAL_OIDC_CLIENT_SECRET`, and add to `config.toml`:

```toml
[identity.oidc]
issuer = "https://auth.example.lan/oidc"
client_id = "corral"
member_group = "corral-users"
admin_group = "corral-admins"

[identity.oidc.opkssh]
client_id = "opkssh"
policy = "24h"
```

The `[identity.oidc.opkssh]` section lets people who signed in that way reach SSH with
`opkssh login` instead of a registered key; [docs/opkssh.md](../../docs/opkssh.md) has what the
issuer's `opkssh` client needs and what users install. For an issuer behind a private CA, mount
the CA into the `corral` container and point `SSL_CERT_FILE` at it.

## Security

- Corral mounts the Docker socket, which gives it control of the host's Docker, and so root on
  the host: it runs sandboxes. It runs as root inside its container because the socket's owner
  and group differ between hosts; a non-root user would not limit what the socket allows.
- The gate faces the internet, so it never holds the socket. It reaches Docker through
  `docker-proxy`, an HAProxy that holds the socket on a network of its own with only the gate on
  it, and lets through exactly what sessions need (`docker-proxy.cfg`): creating an exec in a
  sandbox's container (`corral-<sandbox id>`), and starting, resizing and inspecting that exec.
  The gate cannot create, start, stop or remove containers, read images, volumes or networks,
  mount anything, or exec into Corral's own containers; it can still run any command, as any
  user, in any sandbox. The socket is never mounted into sandboxes. Keep the gate up to date.
- The gate holds no state and no key to any sandbox. For each connection it makes a key, starts
  the overlay's OpenSSH in the sandbox as the sandbox user with that key alone allowed, and talks
  to it over the exec's standard input and output: sandboxes listen on no port.
- Corral's internal listener (port 8081) answers only requests carrying `CORRAL_GATE_SECRET`,
  which Corral and the gate share. It is on the `corral` network alone.
- Sandboxes join the `corral-sandboxes` network, which Corral creates when it is missing; neither
  Corral nor the gate is on it. Every sandbox on it can reach every other sandbox's ports. Users
  have passwordless sudo inside their own sandbox. Both are accepted for a group of trusted users.
- A sandbox's container starts as root, running only the overlay's static `corral-init`, which
  writes the sandbox user and prepares the home; the image's own entrypoint never runs, and
  every session runs as the sandbox user, uid 1000.
- Put the web UI and API behind your existing HTTPS reverse proxy and use an `https://`
  `base_url`: the session cookie is marked `Secure` exactly then. Behind a proxy, set
  `trust_forwarded_for = true` under `[http]`, so password-login limits count each client, not
  the proxy. Over plain HTTP on a LAN the UI works too, copy buttons included.
- The gate accepts public keys and opkssh certificates only, and Corral locks out a source
  address (for IPv6, its /64) after 10 failed connections in 5 minutes. A connection has a
  minute to authenticate and 6 attempts, at most 64 connections wait to authenticate at once, and
  each connection holds at most 64 channels and 16 remote forwards.
- The gate does not speak the PROXY protocol, so behind a TCP tunnel or proxy (such as a
  rathole client container) every client shares the relay's address and one lockout. Put the
  relay and the gate on a Docker network of their own with a fixed `ipam` subnet, and list that
  subnet in `config.toml`; connections from it are then never locked out by address:

  ```toml
  [frontdoor]
  kind = "gate"
  lockout_exempt_networks = ["172.30.0.0/24"]
  ```

  Keys cannot be guessed, so nothing replaces the lockout for those connections. The chart's
  [README](../helm/corral/README.md#lockout-behind-a-tunnel) has a rathole example.

## Operating it

- SSH port: change the published port in `compose.yaml` and `public_port` under `[ssh]` in
  `config.toml` together.
- Upgrade: `docker compose pull && docker compose up -d --wait`. `CORRAL_VERSION` in `.env`
  pins the Corral image tag (`latest` when unset).
- The overlay: Corral pulls `overlay_image` under `[engine]`
  (`ghcr.io/adamaq01/corral-overlay:latest` by default) each time it starts, and uses the copy
  Docker has when the pull fails. Each sandbox mounts the overlay Corral pulled, from its next
  start from sleep on; a running sandbox keeps the one it started with.
- A sandbox running from before the gate, or without the overlay, takes no SSH session until
  its next start: restart it from its page, and the start remakes its container, its home kept.
- `docker compose down` stops Corral and the gate only. Sandbox containers are not part of the
  Compose project: running ones keep running, on the `corral-sandboxes` network.
- Logs: `docker compose logs corral gate`. `RUST_LOG` in `.env` sets the level (`info` by
  default).

## Optional adapters

The defaults are SQLite, local accounts and the Docker engine. Each can be swapped in
configuration; `crates/corral-server/config.example.toml` shows a Kubernetes install with OIDC.

- Postgres: set `DATABASE_URL=postgres://user:password@host:5432/corral` in `.env`.
- OIDC: add an `[identity.oidc]` section with `issuer` and `client_id`, set
  `CORRAL_OIDC_CLIENT_SECRET` in `.env`, and register `<base_url>/auth/oidc/callback` as the redirect
  URI. Members of `member_group` (default `corral-users`) log in as members and members of
  `admin_group` (default `corral-admins`) as admins; anyone else is refused, and a user who
  leaves both groups is disabled at their next login. Local accounts keep working alongside
  it, and the sign-in page offers each login method that is on.
- Vault or OpenBao: add a `[secrets]` section with `kind = "vault"` and `address`, and a
  `[secrets.auth]` section with `kind = "token"`, with `CORRAL_VAULT_TOKEN` in `.env`, to keep
  secret values in a KV v2 mount (`secret` by default) instead of in the database, each at
  `<path_prefix>/<user id>/<value id>`. The token needs `create`, `read`, `update`, `delete` and
  `list` on the prefix's data and metadata paths. The adapter speaks the Vault KV v2 API, which
  HashiCorp Vault and OpenBao both serve.
