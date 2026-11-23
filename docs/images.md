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
- On Docker, the overlay's OpenSSH server, which ContainerSSH connects to, and on both engines
  the overlay's `sftp-server`.
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
written at every start.

## How each engine starts a sandbox

Docker (Docker Engine 28 or later, for image mounts): the container mounts the overlay as an image
mount pinned to the image ID Corral pulled when it started, runs as root
`/opt/corral/bin/tini -- /opt/corral/bin/corral-init docker`, which writes the accounts, sudo rule,
and profile, prepares the home as the sandbox user, then runs the overlay's sshd.

Kubernetes (1.35 or later, for image volumes): the pod mounts the overlay as an image volume. A
root init container runs `corral-init prepare`, which writes the account files into an emptyDir
and prepares the home as the sandbox user. The sandbox container runs as uid 1000 under tini, with
those files mounted over the image's `/etc/passwd`, `/etc/group`, `/etc/shadow`,
`/etc/sudoers.d/corral-sandbox` and `/etc/profile.d/corral.sh`. ContainerSSH runs
`corral-shell` for logins and the overlay's `sftp-server` for sftp. ContainerSSH runs an SSH
command (`ssh host <command>`) through the image's `/bin/sh`, so an image without `/bin/sh` takes
interactive logins and sftp only.

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
