# Corral on Docker Compose

Two containers run the whole service: Corral (the API, local accounts, SQLite and the Docker
engine) and [ContainerSSH](https://containerssh.io) (the SSH front door). Each sandbox is a Docker
container with its home on a named volume. Users reach a sandbox with any SSH client:

```
ssh -p 48291 <handle>+<sandbox>@<public_host>
```

## Install

You need Docker with the Compose plugin, `ssh-keygen` and `curl`.

1. Copy this directory to the host and keep it private, since `.env` will hold the keys that open
   every sandbox and decrypt every stored credential:

   ```sh
   cp -R deploy/compose /opt/corral && cd /opt/corral && chmod 700 .
   ```

2. In `config.toml`, set `base_url` (the address people open in a browser) and `public_host`
   (the host name people pass to `ssh`).

3. Generate Corral's keys. Run this once: new keys make every stored credential unreadable.

   ```sh
   docker compose run --rm corral keys generate > .env
   ```

4. Generate ContainerSSH's host key. ContainerSSH runs as uid 1022, so the key must be readable
   by others; the private directory from step 1 keeps other host users out.

   ```sh
   ssh-keygen -q -t ed25519 -N '' -C containerssh -f containerssh_host_key && chmod 644 containerssh_host_key
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

The JSON API lives at `base_url`. Log in, register a public key, and create a sandbox:

```sh
curl -c jar -H 'Content-Type: application/json' \
  -d '{"username": "<name>", "password": "<password>"}' <base_url>/auth/local/finish
curl -b jar -H 'Content-Type: application/json' \
  -d "{\"label\": \"laptop\", \"publicKey\": \"$(cat ~/.ssh/id_ed25519.pub)\"}" <base_url>/api/keys
curl -b jar -H 'Content-Type: application/json' -d '{"name": "main"}' <base_url>/api/sandboxes
ssh -p 48291 <name>+main@<public_host>
```

The first sandbox pulls the sandbox image (`ghcr.io/adamaq01/corral-sandbox`, about 2 GB
unpacked), so that request can take a few minutes. Every `POST` and `PUT` needs
`Content-Type: application/json`, including the ones without a body, such as
`POST /api/sandboxes/main/stop`.

- `ssh -p 48291 <name>@<public_host>` opens the default sandbox: the first one created, or the
  one set with `POST /api/sandboxes/<sandbox>/default`.
- Herdr: `hc machine add <name>+main@<public_host>:48291 --name main`.
- Port forwarding works: `ssh -p 48291 -L 3000:127.0.0.1:3000 <name>+main@<public_host>`.
- `PUT /api/credentials/<NAME>` with `{"value": "..."}` stores a credential, encrypted with
  `CORRAL_SECRETS_KEY`. It reaches SSH sessions as an environment variable from the sandbox's
  next wake or `POST /api/sandboxes/<sandbox>/restart` on. Values are never returned.
- Admins add users with `POST /api/admin/local-accounts` and
  `{"username": "...", "password": "...", "role": "member"}`, and disable one with
  `POST /api/admin/users/<id>/disable`.
- A sandbox sleeps (its container stops) after 30 minutes with no SSH session and no Herdr
  terminal at work, and wakes on the next SSH login. Sleep keeps the container, so its files
  survive until the sandbox is deleted.

## Security

- Corral mounts the Docker socket, which gives it control of the host's Docker, and so root on
  the host. It runs as root inside its container because the socket's owner and group differ
  between hosts; a non-root user would not limit what the socket allows. The socket is never
  mounted into sandboxes.
- Sandboxes and ContainerSSH share the `corral-sandboxes` network; Corral is never on it. Corral's
  internal listener (port 8081) answers ContainerSSH with the private key that opens every
  sandbox, so no sandbox can reach it.
- Every sandbox on `corral-sandboxes` can reach every other sandbox's ports. Users have
  passwordless sudo inside their own sandbox. Both are accepted for a group of trusted users.
- Put the web API behind your existing HTTPS reverse proxy and use an `https://` `base_url`: the
  session cookie is marked `Secure` exactly then. Behind a proxy, set `trust_forwarded_for = true`
  under `[http]`, so password-login limits count each client, not the proxy.
- ContainerSSH accepts public keys only, and Corral locks out a source address after 10 failed
  connections in 5 minutes. ContainerSSH's audit log is off.

## Known limitation: one-shot commands

ContainerSSH 0.6.1 relays sessions to Docker sandboxes with its `sshproxy` backend, which can cut
off a command that prints and then exits (`ssh <name>+main@<public_host> <command>`):

- It ends the session as soon as the client's input ends, so `ssh -n` or `< /dev/null` usually
  kills the command before it prints.
- It can close the session before relaying the last of the output. Measured on one host: 11 of
  100 `id -un` runs printed nothing, and 14 of 30 runs printing 200 kB came back short. A
  command that ends with a short pause (`<command>; sleep 0.1`) lost nothing in 100 runs.

Interactive shells, Herdr and `ssh -L` port forwards are not affected: none of them lost data in
the same tests.

## Operating it

- SSH port: change the published port in `compose.yaml` and `public_port` under `[ssh]` in
  `config.toml` together.
- Upgrade: `docker compose pull && docker compose up -d --wait`. `CORRAL_VERSION` in `.env`
  pins the Corral image tag (`latest` when unset).
- `docker compose down` stops Corral and ContainerSSH only. Sandbox containers are not part of
  the Compose project: running ones keep running, and they hold the `corral-sandboxes` network
  until they are deleted through the API.
- Logs: `docker compose logs corral`. `RUST_LOG` in `.env` sets the level (`info` by default).

## Optional adapters

The defaults are SQLite, local accounts and the Docker engine. Each can be swapped in
configuration; `crates/corral-server/config.example.toml` shows every setting.

- Postgres: set `DATABASE_URL=postgres://user:password@host:5432/corral` in `.env`.
- OIDC: add an `[identity.oidc]` section with `issuer` and `client_id`, set
  `CORRAL_OIDC_CLIENT_SECRET` in `.env`, and register `<base_url>/auth/oidc/finish` as the redirect
  URI. Members of `member_group` (default `corral-users`) log in as members and members of
  `admin_group` (default `corral-admins`) as admins; anyone else is refused, and a user who
  leaves both groups is disabled at their next login. Local accounts keep working alongside
  it, and `GET /auth/providers` lists the login methods that are on.
- OpenBao: set `kind = "openbao"` under `[secrets]` and add a `[secrets.openbao]` section with
  `address` and `auth = "token"`, with `CORRAL_OPENBAO_TOKEN` in `.env`, to keep credentials
  there instead of in the database.
