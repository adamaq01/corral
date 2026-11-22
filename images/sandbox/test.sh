#!/usr/bin/env bash
# CORRAL_SANDBOX_IMAGE names a prebuilt image; when unset, corral-sandbox:dev is built from here.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
image=${CORRAL_SANDBOX_IMAGE:-corral-sandbox:dev}
prefix=corral-sandbox-test
network=$prefix
volume=$prefix-home
client=$prefix-client
server=$prefix-sshd

remove_resources() {
  docker rm --force "$client" "$server" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
}
trap remove_resources EXIT
trap 'echo "FAIL: line $LINENO: $BASH_COMMAND" >&2' ERR
remove_resources

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

check() {
  if [[ "$3" != "$2" ]]; then
    fail "$1: expected [$2], got [$3]"
  fi
  echo "ok - $1"
}

if [[ -z "${CORRAL_SANDBOX_IMAGE:-}" ]]; then
  docker build --quiet --tag "$image" "$here" >/dev/null
  echo "built $image"
fi

docker network create "$network" >/dev/null
docker run --detach --name "$client" --network "$network" "$image" >/dev/null

out=$(docker exec "$client" id -u)
check "runs as uid 1000" 1000 "$out"

docker exec "$client" sudo -n true || fail "sudo -n true failed"
echo "ok - passwordless sudo"

for tool in herdr-core hc claude codex; do
  out=$(docker exec "$client" "$tool" --version) || fail "$tool --version failed"
  [[ "$out" =~ [0-9]+\.[0-9]+\.[0-9]+ ]] || fail "$tool --version printed no version: $out"
  echo "ok - $tool --version: $out"
done

docker exec "$client" mkdir -m 700 /home/sandbox/.ssh
docker exec "$client" ssh-keygen -q -t ed25519 -N '' -C proxy -f /home/sandbox/.ssh/proxy_key
docker exec "$client" ssh-keygen -q -t ed25519 -N '' -C host -f /tmp/host_key
host_key=$(docker exec "$client" cat /tmp/host_key)
host_pub=$(docker exec "$client" cat /tmp/host_key.pub)
echo "$server $host_pub" | docker exec --interactive "$client" tee /home/sandbox/.ssh/known_hosts >/dev/null

ssh_run() {
  docker exec "$client" ssh -i /home/sandbox/.ssh/proxy_key \
    -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes \
    -o UserKnownHostsFile=/home/sandbox/.ssh/known_hosts -o GlobalKnownHostsFile=/dev/null \
    "sandbox@$server" "$@"
}

# Stands in for the engine, which writes these files into the stopped container before each start.
docker exec "$client" sh -c '
  mkdir -p /tmp/session/etc/ssh /tmp/session/etc/corral
  cp /home/sandbox/.ssh/proxy_key.pub /tmp/session/etc/ssh/corral_authorized_keys
  echo "SetEnv \"FOO_TOKEN=bar\"" > /tmp/session/etc/corral/sshd_env.conf
  chmod 0644 /tmp/session/etc/ssh/corral_authorized_keys
  chmod 0600 /tmp/session/etc/corral/sshd_env.conf
'

start_server() {
  docker create --name "$server" --network "$network" --volume "$volume:/home/sandbox" \
    --env CORRAL_SSHD=1 --env CORRAL_SSH_HOST_KEY="$host_key" "$image" >/dev/null
  docker exec "$client" tar -C /tmp/session --owner=0 --group=0 -cf - \
    etc/ssh/corral_authorized_keys etc/corral/sshd_env.conf | docker cp - "$server:/"
  docker start "$server" >/dev/null
  for _ in $(seq 50); do
    if ssh_run true 2>/dev/null; then
      return
    fi
    if [[ "$(docker inspect --format '{{.State.Running}}' "$server")" != true ]]; then
      docker logs "$server" >&2
      fail "sshd container exited"
    fi
    sleep 0.2
  done
  docker logs "$server" >&2
  ssh_run true || fail "ssh to $server never succeeded"
}

start_server

out=$(ssh_run id -un)
check "ssh session runs as sandbox" sandbox "$out"

out=$(ssh_run printenv FOO_TOKEN)
check "the written SetEnv reaches the ssh session" bar "$out"

out=$(ssh_run 'printenv CORRAL_SSH_HOST_KEY; env | grep ^CORRAL_; true')
check "no CORRAL_ variable reaches the ssh session" "" "$out"

out=$(ssh_run 'test -e ~/.ssh/authorized_keys && echo present || echo absent')
check "home has no authorized_keys" absent "$out"

out=$(ssh_run 'test -f ~/.corral-initialized && test -f ~/.bashrc && echo initialized')
check "first start copies the skeleton home" initialized "$out"

ssh_run 'echo kept > ~/persist-check && echo "# edited" >> ~/.bashrc'
docker rm --force "$server" >/dev/null
start_server

out=$(ssh_run cat persist-check)
check "home file survives a second start on the same volume" kept "$out"

out=$(ssh_run tail -n 1 .bashrc)
check "second start leaves the initialized home alone" "# edited" "$out"

echo "all checks passed"
