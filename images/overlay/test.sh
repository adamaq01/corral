#!/usr/bin/env bash
# Starts base images the way the Docker engine starts a sandbox, with the overlay as an image
# mount, and checks each is a whole Corral sandbox over SSH, reached the way the gate reaches it:
# the overlay's corral-sshd, run by an exec as the sandbox user.
# CORRAL_OVERLAY_IMAGE names a prebuilt overlay; when unset, this checkout's is built
# (tests/checkout.sh). Its containers, volume and images are named after CORRAL_RUN_ID.
# CORRAL_OVERLAY_TEST_BASES lists the base images, space-separated, to check instead of the default.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
source "$repo/tests/checkout.sh"
overlay=$CORRAL_OVERLAY_IMAGE
read -r -a bases <<<"${CORRAL_OVERLAY_TEST_BASES:-ubuntu:24.04 debian:trixie-slim alpine:3.24 gcr.io/distroless/static-debian13}"
prefix=corral-overlay-test-$CORRAL_RUN_ID
volume=$prefix-home
server=$prefix-sandbox
baked=$prefix-baked:test
groups=$prefix-groups:test
with_git=$prefix-git:test
work=$(mktemp -d "${TMPDIR:-/tmp}/corral-overlay-test.XXXXXX")

remove_resources() {
  docker rm --force "$server" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
  docker rmi "$baked" "$groups" "$with_git" >/dev/null 2>&1 || true
}
trap 'remove_resources; rm -rf "$work"' EXIT
trap 'echo "FAIL: line $LINENO: $BASH_COMMAND" >&2' ERR
remove_resources

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

base=
check() {
  if [[ "$3" != "$2" ]]; then
    fail "$base: $1: expected [$2], got [$3]"
  fi
  echo "ok - $base: $1"
}

"$repo/tests/images.sh" overlay

# What the gate does for each connection: a fresh key, and corral-sshd run with its public half.
ssh-keygen -q -t ed25519 -N '' -C gate -f "$work/key"
public_key=$(cut -d' ' -f1,2 "$work/key.pub")
# The session variables, as the engine writes them before each start.
mkdir -p "$work/session/etc/corral"
echo 'SetEnv "FOO_TOKEN=bar"' >"$work/session/etc/corral/sshd_env.conf"
chmod 0400 "$work/session/etc/corral/sshd_env.conf"

ssh_options=(-F /dev/null -i "$work/key" -o BatchMode=yes -o IdentitiesOnly=yes
  -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR
  -o "ProxyCommand=docker exec -i -u 1000 $server /opt/corral/libexec/corral-sshd '$public_key'")

ssh_run() {
  ssh "${ssh_options[@]}" sandbox@sandbox "$@"
}

interactive_login() {
  printf 'echo "shell=$SHELL"; exit\n' | ssh -tt "${ssh_options[@]}" sandbox@sandbox 2>/dev/null |
    tr -d '\r'
}

# The home files the engine stages, as corral-init reads them: "<key> <mode> <path>" lines.
stage_home_files() {
  local dir=$work/home-files/etc/corral/home-files
  rm -rf "$work/home-files" && mkdir -p "$dir"
  cat >"$dir/manifest"
  printf "%s" "$1" >"$dir/0"
  printf "plain" >"$dir/1"
  printf "blocked" >"$dir/2"
  chmod 0700 "$dir" && chmod 0600 "$dir"/*
  tar -C "$work/home-files" --owner=1000 --group=1000 --numeric-owner \
    -cf "$work/home-files.tar" etc/corral/home-files
}

# As the Docker engine makes and starts a sandbox's container from the image $1, with any more
# options to docker create after it.
start_server() {
  local image=$1
  shift
  docker create --name "$server" --network none --user 0 \
    --mount "type=volume,source=$volume,target=/home/sandbox" \
    --mount "type=image,source=$overlay,target=/opt/corral" \
    --tmpfs /run/corral:mode=0755 "$@" \
    --entrypoint /opt/corral/bin/tini "$image" -- /opt/corral/bin/corral-init docker >/dev/null
  tar -C "$work/session" --owner=1000 --group=1000 --numeric-owner -cf - etc | docker cp - "$server:/"
  docker cp - "$server:/" <"$work/home-files.tar"
  docker start "$server" >/dev/null
  for _ in $(seq 50); do
    if docker exec "$server" test -e /run/corral/ready && ssh_run true 2>/dev/null; then
      return
    fi
    if [[ "$(docker inspect --format '{{.State.Running}}' "$server")" != true ]]; then
      docker logs "$server" >&2
      fail "$image: the sandbox container exited"
    fi
    sleep 0.2
  done
  docker logs "$server" >&2
  ssh_run true || fail "$image: ssh never succeeded"
}

# The bash a base image's logins run: its own, or the overlay's.
bash_of() {
  docker run --rm --user 0 --entrypoint /opt/corral/bin/sh \
    --mount "type=image,source=$overlay,target=/opt/corral" "$1" -c '
      for shell in /bin/bash /usr/bin/bash /opt/corral/bin/bash; do
        [ -x "$shell" ] && echo "$shell" && break
      done'
}

# As the Kubernetes engine's init container prepares a pod: the account files it mounts.
check_prepare() {
  check "the Kubernetes init container writes the sandbox user into the account files" \
    "sandbox:x:1000:1000:Corral sandbox:/home/sandbox:/opt/corral/bin/corral-shell" \
    "$(docker run --rm --user 0 --entrypoint /opt/corral/bin/sh \
      --mount "type=image,source=$overlay,target=/opt/corral" \
      --mount "type=tmpfs,destination=/home/sandbox,tmpfs-mode=0777" "$base" -c '
        /opt/corral/bin/busybox mkdir /tmp/accounts
        /opt/corral/bin/corral-init prepare /tmp/accounts >/dev/null
        /opt/corral/bin/busybox grep ^sandbox: /tmp/accounts/passwd')"
}

check_base() {
  base=$1
  docker image inspect "$base" >/dev/null 2>&1 || docker pull --quiet "$base" >/dev/null
  stage_home_files first <<'MANIFEST'
0 0600 .config/corral test/secret
1 0644 notes.txt
2 0600 notes.txt/blocked
MANIFEST
  start_server "$base"
  check_prepare

  check "a session runs as the sandbox user, uid 1000" "sandbox 1000" "$(ssh_run 'echo "$(id -un) $(id -u)"')"
  check "uid 1000 is the sandbox user alone" "sandbox" \
    "$(ssh_run "awk -F: '\$3 == 1000 { print \$1 }' /etc/passwd")"
  check "passwordless sudo" root "$(ssh_run 'sudo -n id -un')"
  check "the login shell is a bash" bash "$(ssh_run 'basename "$SHELL"')"
  check "a command's SHELL is the bash a login runs" "$(bash_of "$base")" "$(ssh_run 'echo "$SHELL"')"
  check "the sandbox user is in its own group alone" "1000" "$(ssh_run 'id -G')"
  check "the written SetEnv reaches the session" bar "$(ssh_run 'printenv FOO_TOKEN')"
  check "no CORRAL_ variable reaches the session" "" "$(ssh_run 'env | grep ^CORRAL_; true')"
  check "corral is on PATH" "corral acts on the Corral sandbox it runs in." \
    "$(ssh_run 'corral help | head -n 1')"
  [[ "$(ssh_run 'hc --version')" =~ [0-9]+\.[0-9]+\.[0-9]+ ]] || fail "$base: hc --version"
  echo "ok - $base: hc is on PATH"

  local login
  login=$(interactive_login)
  check "an interactive login prints the motd once" 1 "$(grep -c 'This is a Corral sandbox' <<<"$login" || true)"
  [[ "$login" == *"shell=/"*bash* ]] || fail "$base: an interactive login: $login"
  check "a command prints only its output" "only this" "$(ssh_run 'echo only this')"

  echo "put $work/key.pub uploaded" >"$work/batch"
  sftp -q -b "$work/batch" "${ssh_options[@]}" sandbox@sandbox >/dev/null
  check "sftp writes into the home" "$(cat "$work/key.pub")" "$(ssh_run 'cat ~/uploaded')"
  check "no sshd runs in the sandbox" "" \
    "$(docker exec "$server" /opt/corral/bin/busybox ps -o args | grep -E 'listener|sshd -D' || true)"

  check "the first start copies a skeleton home" initialized \
    "$(ssh_run 'test -f ~/.corral-initialized && test -f ~/.profile && echo initialized')"
  check "a staged home file is written with its mode, by the sandbox user, its directories made" \
    "first
600 sandbox
755 sandbox
644 sandbox" \
    "$(ssh_run 'cat ~/".config/corral test/secret"; echo; stat -c "%a %U" ~/".config/corral test/secret" ~/".config/corral test" ~/notes.txt')"
  check "a staged file that cannot be written is skipped" plain "$(ssh_run 'cat ~/notes.txt')"
  check "a file it could not write is reported at login and by corral status" 1 \
    "$(grep -c 'Corral could not write ~/notes.txt/blocked' <<<"$login" || true)"
  [[ "$(ssh_run 'cat ~/.config/corral/notices')" == *"~/notes.txt/blocked"* ]] ||
    fail "$base: the notice of an unwritten file"
  echo "ok - $base: corral status reads the notices corral-init writes"

  check "the activity reporter runs as the sandbox user" sandbox \
    "$(docker exec "$server" /opt/corral/bin/busybox ps -o user,args |
      awk '$2 == "/opt/corral/bin/corral" && $3 == "report-activity" { print $1 }')"
  check "the sandbox's own cgroup and network counters are readable as uid 1000" "1 yes" \
    "$(docker exec --user 1000 "$server" /opt/corral/bin/sh -c \
      'echo $(/opt/corral/bin/busybox grep -c ^usage_usec /sys/fs/cgroup/cpu.stat) $(/opt/corral/bin/busybox grep -q lo: /proc/net/dev && echo yes)')"
  for _ in $(seq 25); do
    docker logs "$server" 2>&1 | grep -q 'corral report-activity: ' && break
    sleep 0.2
  done
  [[ "$(docker logs "$server" 2>&1)" == *"corral report-activity: "*"sandbox.json"* ]] ||
    fail "$base: the reporter did not say it has no sandbox file to report with"
  echo "ok - $base: with no sandbox file, the reporter says so and keeps trying"


  ssh_run 'echo kept > ~/kept && echo "edited inside" > ~/".config/corral test/secret"'
  stage_home_files second <<'MANIFEST'
0 0600 .config/corral test/secret
1 0644 notes.txt
2 0644 unchanged
MANIFEST
  docker rm --force "$server" >/dev/null
  start_server "$base"
  check "a new container keeps the home and replaces the staged files" "kept
second
plain" "$(ssh_run 'cat ~/kept; cat ~/".config/corral test/secret"; echo; cat ~/notes.txt')"
  check "nothing to report leaves no notices" absent "$(ssh_run 'test -e ~/.config/corral/notices || echo absent')"

  stage_home_files third <<'MANIFEST'
0 0600 .config/corral test/secret
MANIFEST
  docker rm --force "$server" >/dev/null
  start_server "$base"
  check "a file Corral wrote and no longer stages is removed" gone \
    "$(ssh_run 'test -e ~/unchanged || test -e ~/notes.txt || echo gone')"
  check "a file Corral never wrote stays" kept "$(ssh_run 'cat ~/kept')"

  ssh_run 'echo "edited inside" > ~/".config/corral test/secret"'
  stage_home_files fourth </dev/null
  docker rm --force "$server" >/dev/null
  start_server "$base"
  check "a file changed in the sandbox is left and reported" "edited inside
~/.config/corral test/secret is no longer attached, but it was changed in the sandbox, so Corral left it." \
    "$(ssh_run 'cat ~/".config/corral test/secret"; cat ~/.config/corral/notices')"

  docker rm --force "$server" >/dev/null
  docker volume rm "$volume" >/dev/null
}

# The sandbox file Corral writes, naming the repository $1, as the only staged home file.
stage_repository() {
  stage_home_files "{\"api\": \"http://corral.invalid\", \"web\": \"http://corral.invalid\",
    \"sandbox\": \"main\", \"token\": \"corral_sbx_test\",
    \"repository\": {\"url\": \"$1\", \"branch\": null}}" <<<'0 0600 .config/corral/sandbox.json'
}

# A bare repository of one commit, mounted read-only where the sandbox file names it.
make_repository() {
  local git=(env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -c user.name=corral
    -c user.email=corral@example.com)
  "${git[@]}" init -q -b main "$work/source"
  echo hello >"$work/source/README"
  "${git[@]}" -C "$work/source" add README
  "${git[@]}" -C "$work/source" commit -q -m first
  "${git[@]}" clone -q --bare "$work/source" "$work/repo.git"
  chmod -R a+rX "$work/repo.git"
}

# The first start clones the repository the sandbox file names, with the image's git.
check_repository() {
  local repository=(--mount "type=bind,source=$work/repo.git,target=/srv/repo.git,readonly")
  base=$with_git
  docker build --quiet --tag "$with_git" - >/dev/null <<'EOF2'
FROM alpine:3.24
RUN apk add --no-cache git
EOF2
  make_repository

  stage_repository file:///srv/repo.git
  start_server "$with_git" "${repository[@]}"
  check "the first start clones the repository into the home" "hello
Cloned file:///srv/repo.git into ~/repo" \
    "$(ssh_run 'test -d ~/repo/.git && cat ~/repo/README ~/.config/corral/notices')"
  ssh_run 'rm -rf ~/repo'
  docker rm --force "$server" >/dev/null
  start_server "$with_git" "${repository[@]}"
  check "a later start clones nothing" "absent absent" \
    "$(ssh_run 'echo $(test -e ~/repo || echo absent) $(test -e ~/.config/corral/notices || echo absent)')"
  docker rm --force "$server" >/dev/null
  docker volume rm "$volume" >/dev/null

  stage_repository https://nowhere.invalid/x.git
  start_server "$with_git"
  [[ "$(ssh_run 'cat ~/.config/corral/notices')" == "Cloning https://nowhere.invalid/x.git failed: "?* ]] ||
    fail "$base: the notice of a failed clone: $(ssh_run 'cat ~/.config/corral/notices')"
  echo "ok - $base: a failed clone is a notice and the sandbox starts"
  [[ "$(interactive_login | grep -A1 "^At this sandbox's last start:" | tail -n 1)" == \
    "  Cloning https://nowhere.invalid/x.git failed: "?* ]] || fail "$base: the login's notices"
  echo "ok - $base: an interactive login prints the failed clone under the last start"
  docker rm --force "$server" >/dev/null
  docker volume rm "$volume" >/dev/null

  base=alpine:3.24
  stage_repository file:///srv/repo.git
  start_server "$base" "${repository[@]}"
  check "an image without git starts and says it cannot clone" \
    "Cloning file:///srv/repo.git needs git, which this image does not have" \
    "$(ssh_run 'cat ~/.config/corral/notices; test -e ~/repo && echo cloned; true')"
  docker rm --force "$server" >/dev/null
  docker volume rm "$volume" >/dev/null
}

for each in "${bases[@]}"; do
  check_base "$each"
done
check_repository

# An image whose own uid 1000, and a group listing sandbox, would give the sandbox user more
# groups.
docker build --quiet --tag "$groups" - >/dev/null <<'EOF2'
FROM alpine:3.24
RUN adduser -D -u 1000 olduser && addgroup -g 1500 extra && addgroup olduser extra \
 && addgroup olduser wheel && echo 'tools:x:1600:sandbox' >> /etc/group
EOF2
check_base "$groups"

# An image that bakes the overlay in still starts the same way.
base=recipe
docker build --quiet --tag "$baked" - >/dev/null <<EOF
FROM alpine:3.24
USER root
COPY --from=$overlay / /opt/corral/
RUN ["/opt/corral/bin/corral-init", "build"]
USER 1000
EOF
check "the build makes the sandbox user and its home" "sandbox:x:1000:1000:Corral sandbox:/home/sandbox:/opt/corral/bin/corral-shell 1000" \
  "$(docker run --rm --user 0 --entrypoint /opt/corral/bin/sh "$baked" -c 'grep ^sandbox: /etc/passwd; stat -c %u /home/sandbox' | tr '\n' ' ' | sed 's/ $//')"
check_base "$baked"

echo "all checks passed"
