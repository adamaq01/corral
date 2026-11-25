# Sandbox images

A sandbox can run any Linux image: Ubuntu, Debian, Alpine, a distroless image or one of your own.
Nothing in the image needs to know about Corral. What makes it a Corral sandbox comes from the
Corral overlay, `ghcr.io/adamaq01/corral-overlay`, which every sandbox mounts read-only at
`/opt/corral`. No image is built for this: the overlay is mounted when the sandbox starts.

The catalog's default image, `ghcr.io/adamaq01/corral-sandbox`, is Ubuntu 24.04 with Node.js,
Claude Code, Codex, Python 3, git, curl and build tools, and nothing of Corral's own.

## What the overlay brings

Every binary in the overlay is static, so it runs whatever libc the image has, or none:

- `corral-init`, which runs before anything else and writes the sandbox user: `sandbox`, uid and
  gid 1000, home `/home/sandbox`. Any user or group the image has with id 1000 is replaced.
- Passwordless sudo: the overlay's sudo (built without PAM), and a rule in
  `/etc/sudoers.d/corral-sandbox` for an image's own sudo.
- `corral-shell`, the sandbox user's login shell: the image's bash, or the overlay's when the image
  has none, as a login shell. It prints the motd before an interactive login.
- OpenSSH's `sshd` and `sftp-server`, which `corral-sshd` runs for each SSH connection the
  gate opens: `sshd -i` as the sandbox user, over the exec's standard input and output, with the
  gate's key for that connection alone allowed and a host key the overlay ships, which the gate
  does not check since it reached the sandbox through the engine. Nothing in a sandbox listens
  for SSH.
- Herdr's `herdr-core` and `hc`, and the `corral` command (`corral status`, `restart`, `stop`,
  `delete --confirm`).
- busybox, whose commands come last on `PATH`, after the image's own, so an image without `ls` or
  `ps` still has them.
- tini, which runs as PID 1 and reaps orphaned processes.
- A home skeleton, copied into an empty home on the first start, unless the image has a
  non-empty `/etc/skel`, which is copied instead.

The image's own entrypoint and command never run. The sandbox starts as root only for
`corral-init`, which runs the overlay's static code alone, and every session runs as `sandbox`.

Sessions get the same `PATH` on both engines:
`/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/opt/corral/bin:/opt/corral/fallback`,
reset by the image's `/etc/profile` in login shells where it sets its own. An image's `ENV PATH`
does not reach sessions, so install tools into one of those directories, or add them to `PATH` in
a file under `/etc/profile.d/`.

Only `/home/sandbox` survives sleep. The home files and variables of a sandbox's environment are
written at every start. `corral-init` records what it wrote in `~/.config/corral/home-files`,
removes a file it wrote earlier that is no longer attached unless it was changed since, and lists
what it could not do in `~/.config/corral/notices`, which the motd of an interactive login and
`corral status` show.

Sessions run through OpenSSH on both engines, so they get its environment: `USER`, `HOME`,
`SHELL` and the rest from the sandbox user's account, and the sandbox's variables, which the
engine writes to `/etc/corral/sshd_env.conf` at every start as sshd `SetEnv`. `corral-shell` then
names the bash a login runs in `SHELL`, the image's when it has one, else the overlay's. A client
may send `LANG`, `LC_*`, `COLORTERM` and `NO_COLOR`. The sandbox user is in its own group alone,
whatever the image's `/etc/group` lists for uid 1000 or for `sandbox`.

## Who picks the image

Admins list images in the catalog (`[[catalog.images]]` in `config.toml`, `catalog.images` in the
chart's values), and users pick one when they create a sandbox. A change to an entry's reference
reaches each sandbox made from it at its next start from sleep.

Users may also name an image of their own, from registries the admin allows and up to a size the
admin sets. It is off until the catalog says so:

```toml
[catalog.named_images]
# Registries, such as ghcr.io, or repository paths in one, such as ghcr.io/acme.
registries = ["docker.io", "ghcr.io/acme"]
# The most an image may take to download, its layers compressed, for Corral's own platform.
max_size = "5Gi"
```

The web UI then offers **Another image** beside the catalog's, and the API takes
`imageReference` in place of `image` when a sandbox is created. Corral reads the image's manifest
from its registry before it makes the sandbox, and refuses an image from another registry, one
that takes more than `max_size` to download, or one it cannot read.

## Private registries

Credentials for a registry are the admin's and never shown to users. For the size check of named
images, and on Docker for every pull, Corral uses those under `[[registries]]`; the password
comes from the environment variable `password_env` names:

```toml
[[registries]]
registry = "registry.example.lan:5000"
username = "corral"
password_env = "CORRAL_REGISTRY_PASSWORD"
# Read the size of named images over plain HTTP.
plain_http = true
```

On Kubernetes, nodes pull with Secrets of type `kubernetes.io/dockerconfigjson` in the sandbox
namespace, named under `image_pull_secrets` in `[engine]` (`sandboxes.imagePullSecrets` in the
chart); every sandbox pod uses them, for its image and the overlay. The chart's `registries` value
holds the `[[registries]]` entries, each password a secret value.

## How each engine starts a sandbox

Docker (Docker Engine 28 or later, for image mounts): the container mounts the overlay as an image
mount pinned to the image ID Corral pulled when it started, runs as root
`/opt/corral/bin/tini -- /opt/corral/bin/corral-init docker`, which writes the accounts, sudo rule,
and profile, prepares the home as the sandbox user, then marks the sandbox ready in a tmpfs at
`/run/corral` and waits. Each SSH connection is a `docker exec` of `corral-sshd` as uid 1000.

Kubernetes (1.35 or later, for image volumes): the pod mounts the overlay as an image volume. A
root init container runs `corral-init prepare`, which writes the account files into an emptyDir
and prepares the home as the sandbox user. The sandbox container runs as uid 1000 under tini, with
those files mounted over the image's `/etc/passwd`, `/etc/group`, `/etc/shadow`,
`/etc/sudoers.d/corral-sandbox` and `/etc/profile.d/corral.sh`, and the sandbox's variables
mounted at `/etc/corral/sshd_env.conf`. Each SSH connection is a pod exec of `corral-sshd`.

A new overlay reaches a sandbox at its next start from sleep: Docker remakes the container, its
home kept, and Kubernetes starts the pod from a new template.

## Baking the overlay in

An image can carry the overlay itself, for builds that need the sandbox user, such as files owned
by it or tools installed into `/etc/skel`:

```dockerfile
FROM debian:13
USER root
COPY --from=ghcr.io/adamaq01/corral-overlay / /opt/corral/
RUN ["/opt/corral/bin/corral-init", "build"]
```

`corral-init build` writes the sandbox user, its sudo rule and its `PATH`, and makes
`/home/sandbox`. Corral still mounts its overlay over `/opt/corral` at every start, so the baked
copy serves the build alone.
