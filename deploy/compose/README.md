# Corral on Docker Compose

Two containers run the whole service: Corral (the web UI and API, local accounts, SQLite and the
Docker engine) and [ContainerSSH](https://containerssh.io) (the SSH front door). Each sandbox is a
Docker container with its home on a named volume, from any image: the Corral overlay, mounted into
every sandbox, brings the sandbox user, sudo, SSH, Herdr and the `corral` command
([docs/images.md](../../docs/images.md)). Users reach a sandbox with any SSH client:

```
ssh -p 48291 <sandbox>.<handle>@<public_host>
```

A bare `<handle>` reaches the user's default sandbox. A sandbox name the user has not used yet
makes that sandbox, from the image and size set in their profile, then opens it, unless they
already have as many as `max_sandboxes_per_user` under `[limits]` allows (5 by default).

The ContainerSSH image is [a build](https://github.com/adamaq01/ContainerSSH) of ContainerSSH's
main branch with two fixes not yet released upstream: one-shot commands (`ssh -n <command>`) keep
all their output, and Kubernetes terminals get no stray bytes and end when the connection drops.

## Install

You need Docker Engine 28 or later, whose image mounts bring the overlay into sandboxes, with the
Compose plugin, and `ssh-keygen`.

1. Copy this directory to the host and keep it private, since it will hold the key that opens
   every sandbox and the key that decrypts every stored secret:

   ```sh
   cp -R deploy/compose /opt/corral && cd /opt/corral && chmod 700 .
   ```

2. In `config.toml`, set `base_url` (the address people open in a browser) and `public_host`
   (the host name people pass to `ssh`).

3. Generate Corral's keys. Run this once: new keys make every stored secret unreadable.

   ```sh
   docker compose run --rm corral keys generate > .env
   ```

4. Generate ContainerSSH's host key and the key it opens sandboxes with, and give Corral the
   public half of the second, which every sandbox trusts. ContainerSSH runs as uid 1022, so both
   keys must be readable by others; the private directory from step 1 keeps other host users out.

   ```sh
   for key in containerssh_host_key containerssh_proxy_key; do
     ssh-keygen -q -t ed25519 -N '' -C containerssh -f "$key" && chmod 644 "$key"
   done
   echo "CORRAL_PROXY_PUBLIC_KEY=$(cat containerssh_proxy_key.pub)" >> .env
   ```

   `ssh-keygen -lf containerssh_host_key.pub` prints the fingerprint users see on first connect.

5. Start both services and wait until they are healthy:

   ```sh
   docker compose up -d --wait
   ```

6. Create the first admin. The password is prompted for twice, at least 12 characters:

   ```sh
   docker compose exec corral corral-server local-account create <name> --admin
   ```

## First sandbox

Open `base_url` in a browser and sign in with the admin account from step 6. Then:

1. Under **SSH keys**, add your public key, such as the contents of `~/.ssh/id_ed25519.pub`.
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
- Your **Profile** sets the image and size a new sandbox gets unless you pick others, including
  one made by connecting as `<sandbox>.<name>`. Until you set them, they are the first image and
  size in `config.toml`'s catalog.
- Users can also name an image of their own once `[catalog.named_images]` lists the registries
  it may come from and the most it may take to download; credentials for a private registry go
  under `[[registries]]`, never shown to users. [docs/images.md](../../docs/images.md) has both.
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
  running sandbox shows **Restart to apply** until then.
- Admins add, reset and delete local accounts under **Local accounts**, and disable, enable or
  delete users under **Users**. **All sandboxes** lists every user's sandboxes.
- Deleting a user, as an admin or from your own **Profile**, deletes their sandboxes with their
  homes, keys, environment and local account. The username is free again afterwards, and a new
  account with it starts empty. A deletion that fails part way leaves the user with status
  Deleting, unable to log in, and deleting them again finishes it.
- A sandbox sleeps (its container stops) after 30 minutes with no SSH session and no Herdr
  terminal at work (`timeout_secs` under `[idle]`), and wakes on the next SSH login. Sleep keeps
  the container, so its files survive until the sandbox is deleted.

### From inside a sandbox

The `corral` command acts on the sandbox it runs in:

- `corral status` shows the sandbox and its state.
- `corral restart` and `corral stop` restart or stop it. Corral accepts the request and does it
  next, so the session that asked ends as the sandbox stops.
- `corral delete --confirm <sandbox>` stops the sandbox at once and deletes it, with its home, 10
  minutes later. It is refused until you turn it on in your **Profile**. Until the deletion, the
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
  the host. It runs as root inside its container because the socket's owner and group differ
  between hosts; a non-root user would not limit what the socket allows. The socket is never
  mounted into sandboxes.
- Only ContainerSSH holds the private key that opens every sandbox (`containerssh_proxy_key`);
  Corral keeps its public half, and its answers to ContainerSSH never carry the key.
- Sandboxes and ContainerSSH share the `corral-sandboxes` network; Corral is never on it. Corral's
  internal listener (port 8081) is unauthenticated and wakes any sandbox it is asked about, so no
  sandbox can reach it.
- Every sandbox on `corral-sandboxes` can reach every other sandbox's ports. Users have
  passwordless sudo inside their own sandbox. Both are accepted for a group of trusted users.
- A sandbox's container starts as root, running only the overlay's static `corral-init`, which
  writes the sandbox user and starts sshd; the image's own entrypoint never runs, and every
  session runs as the sandbox user, uid 1000.
- Put the web UI and API behind your existing HTTPS reverse proxy and use an `https://`
  `base_url`: the session cookie is marked `Secure` exactly then. Behind a proxy, set
  `trust_forwarded_for = true` under `[http]`, so password-login limits count each client, not
  the proxy. Over plain HTTP on a LAN the UI works too, copy buttons included.
- ContainerSSH accepts public keys only, and Corral locks out a source address (for IPv6, its
  /64) after 10 failed connections in 5 minutes. ContainerSSH's audit log is off.
- ContainerSSH does not speak the PROXY protocol, so behind a TCP tunnel or proxy (such
  as a rathole client container) every client shares the relay's address and one lockout. Put
  the relay and ContainerSSH on a Docker network of their own with a fixed `ipam` subnet, and
  list that subnet in `config.toml`; connections from it are then never locked out by address:

  ```toml
  [frontdoor]
  kind = "containerssh"
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
- A sandbox running without the overlay, started by a Corral from before sandboxes mounted it,
  cannot be checked for activity, so it stays awake until it is stopped or restarted; that start
  remakes its container with the overlay, its home and host key kept.
- `docker compose down` stops Corral and ContainerSSH only. Sandbox containers are not part of
  the Compose project: running ones keep running, and they hold the `corral-sandboxes` network
  until they are deleted in the web UI.
- Logs: `docker compose logs corral`. `RUST_LOG` in `.env` sets the level (`info` by default).

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
