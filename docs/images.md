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
`corral-init`, which runs nothing but the overlay's static code as root, and every session runs as
`sandbox`.

Sessions get the same `PATH` on both engines:
`/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/opt/corral/bin:/opt/corral/fallback`,
reset by the image's `/etc/profile` in login shells where it sets its own. An image's `ENV PATH`
does not reach sessions, so install tools into one of those directories, or add them to `PATH` in
a file under `/etc/profile.d/`.

Only `/home/sandbox` survives sleep. The home files and variables of a sandbox's setup are written
at every start. `corral-init` records what it wrote in `~/.config/corral/home-files`,
removes a file it wrote earlier that is no longer attached unless it was changed since, and lists
what it could not do in `~/.config/corral/notices`, which the motd of an interactive login and
`corral status` show.

Every start then applies the setup's MCP servers and gateway addresses to the agents a sandbox
may run: Claude Code, Codex, Pi and oh-my-pi. The MCP servers are those of
`~/.config/corral-agents/mcp.json`, an object of servers by name, each a command (`command`,
`args`, `env`) or a URL (`"type": "http"`, `url`, `headers`). They replace `mcpServers` in
`~/.claude.json`, `~/.pi/agent/mcp.json` and `~/.omp/agent/mcp.json`, and `[mcp_servers]` in
`~/.codex/config.toml`, where a URL's headers are `http_headers`. Without that file, the agents'
MCP servers stay as they are. The addresses are the sandbox's `ANTHROPIC_BASE_URL` and
`OPENAI_BASE_URL` variables, which Claude Code and oh-my-pi read themselves. Codex gets
`OPENAI_BASE_URL` as `openai_base_url` in `~/.codex/config.toml`, and Pi gets each address as
`providers.anthropic.baseUrl` or `providers.openai.baseUrl` in `~/.pi/agent/models.json`. Once a
variable is gone, its key is removed, and so is a provider that the removal leaves empty.
Everything else in these files stays as the agents wrote it, a file is written only when one of
these keys changes in it, and a file that is not valid JSON or TOML is left alone. One line in the
notices names the agents that got the servers, one the agents that got the address, and one says
why for each file they could not be applied to, and for an `mcp.json` that is not valid JSON.

The first start, the one that finds no `~/.corral-initialized`, copies the skeleton, writes the
home files, checks out the setup's dotfiles, if any, then clones the repository the sandbox was
made with, if any, into `~/<name>`, the last segment of its URL without `.git`. The dotfiles are
the repository `~/.config/corral-agents/dotfiles.json` names, as `{"url": "...", "branch": "..."}`
with `null` for the repository's default branch. It is cloned bare into
`~/.config/corral-agents/dotfiles.git`, which stays, and checked out with the home as its work
tree: each of its files replaces what the skeleton or a home file put at its path, and the next
start writes that home file again. Git runs as `sandbox` with the image's own `git` and `PATH`,
never prompts, accepts the SSH host key of a host it has not seen, and stops after 5 minutes. Both
URLs take the same shapes: `https://` without a login, `ssh://` without a password, `file:///`, or
`user@host:path`. A private repository needs its token or key among the setup's files, such as
`~/.git-credentials` with a `~/.gitconfig` that reads it, or a key in `~/.ssh`: the setup's
variables reach sessions, not git. One line in the notices says whether the dotfiles were checked
out, and one whether the repository was cloned, and if not, why: a `dotfiles.json` that is not
valid or a URL of another shape, `dotfiles.git` or the repository's directory exists already, the
image has no `git`, or git's last `fatal:` line, else its last line of output. A failed checkout
removes the `dotfiles.git` it cloned. Later starts never check out or clone.

Sessions run through OpenSSH on both engines, so they get its environment: `USER`, `HOME`,
`SHELL` and the rest from the sandbox user's account, and the sandbox's variables, which the
engine writes to `/etc/corral/sshd_env.conf` at every start as sshd `SetEnv`. `corral-shell` then
names the bash a login runs in `SHELL`, the image's when it has one, else the overlay's. A client
may send `LANG`, `LC_*`, `COLORTERM` and `NO_COLOR`. The sandbox user is in its own group alone,
whatever the image's `/etc/group` lists for uid 1000 or for `sandbox`.

## Who picks the image

Admins list images in the catalog, under **Settings** > **Catalog** in the web UI, and users pick
one when they create a sandbox. The config file can list images too (`[[catalog.images]]` in
`config.toml`, `catalog.images` in the chart's values). Settings shows those locked, and they
change in the file alone. A change to an entry's reference reaches each sandbox made from it at
its next start from sleep.

Users may also name an image of their own, from registries the admin allows and up to a size the
admin sets. It is off until the admin turns it on under **Named images** in Settings > Catalog.
The config file can set it instead, and Settings then shows it locked:

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

A registry's login is the admin's and never shown to users. For the size check of named
images, and on Docker for every pull, Corral logs in with the registries set under **Settings** >
**Registries** or in the config file under `[[registries]]`. Settings shows the config file's
registries locked:

```toml
[[registries]]
registry = "registry.example.lan:5000"
username = "corral"
password_env = "CORRAL_REGISTRY_PASSWORD"
# Read the size of named images over plain HTTP.
plain_http = true
```

A login names the environment variable that holds its password (`password_env` in the config
file), and the variable is set in the deployment, beside Corral's other secrets. Corral reads it
each time it reaches the registry. While it is unset, Corral reaches the registry without a login
and logs a warning, and Settings shows **Variable not set** beside the registry.

When the config file sets an image or size key, or a registry host, that Settings already keeps,
the entry kept in Settings is shadowed: it has no effect, Settings marks it **Shadowed by
config**, and the next save of that page drops it.

On Kubernetes, nodes pull with Secrets of type `kubernetes.io/dockerconfigjson` in the sandbox
namespace, named under `image_pull_secrets` in `[engine]` (`sandboxes.imagePullSecrets` in the
chart); every sandbox pod uses them, for its image and the overlay. The chart's `registries` value
holds the `[[registries]]` entries, each password a secret value, and its `registryPasswords`
value sets the variables that the registries added in Settings name.

## How each engine starts a sandbox

Docker (Docker Engine 28 or later, for image mounts): the container mounts the overlay as an image
mount pinned to the image ID Corral pulled when it started, runs as root
`/opt/corral/bin/tini -- /opt/corral/bin/corral-init docker`, which writes the accounts, sudo rule,
and profile, prepares the home as the sandbox user, then marks the sandbox ready in a tmpfs at
`/run/corral` and waits. Each SSH connection is a `docker exec` of `corral-sshd` as uid 1000.

Kubernetes (1.35 or later, for image volumes): the pod mounts the overlay as an image volume, and
the sandbox's variables at `/etc/corral/sshd_env.conf` in both of its containers. A root init
container runs `corral-init prepare`, which writes the account files into an emptyDir and prepares
the home as the sandbox user. The sandbox container runs as uid 1000 under tini, with those files
mounted over the image's `/etc/passwd`, `/etc/group`, `/etc/shadow`,
`/etc/sudoers.d/corral-sandbox` and `/etc/profile.d/corral.sh`. Each SSH connection is a pod exec
of `corral-sshd`.

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
