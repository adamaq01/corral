# Corral Helm chart

Runs Corral on Kubernetes, with [agent-sandbox](https://github.com/kubernetes-sigs/agent-sandbox)
as the sandbox engine and [ContainerSSH](https://containerssh.io) as the SSH front door. Each
sandbox is an agent-sandbox `Sandbox` whose home is a PersistentVolumeClaim, from any image: the
Corral overlay, mounted into every sandbox, brings the sandbox user, sudo, the login shell, sftp,
Herdr and the `corral` command ([docs/images.md](../../../docs/images.md)). Users reach a sandbox
with any SSH client:

```
ssh -p 48291 <sandbox>.<handle>@<ssh.publicHost>
```

A bare `<handle>` reaches the user's default sandbox. A sandbox name the user has not used yet
makes that sandbox, from the image and size set in their profile, then opens it, unless they
already have as many as `limits.maxSandboxesPerUser` allows (5 by default).

The ContainerSSH image is [a build](https://github.com/adamaq01/ContainerSSH) of ContainerSSH's
main branch with two fixes not yet released upstream: one-shot commands (`ssh -n <command>`) keep
all their output, and Kubernetes terminals get no stray bytes and end when the connection drops.

## What it installs

In the release namespace:

- Corral: a Deployment of one replica, a Service for the web UI and API (port 8080), a ClusterIP
  Service `<release>-webhooks` for ContainerSSH's webhooks (port 8081), its config rendered from
  the values, and an optional Ingress or Gateway API HTTPRoute for `baseUrl`'s host.
- ContainerSSH with its Kubernetes backend: public keys only, 10 seconds for Corral to answer
  about a key, port forwarding on, and a config webhook timeout 10 seconds above the wake
  deadline. Its Service is a LoadBalancer by default.
- The database: SQLite on a PersistentVolumeClaim by default, an existing Postgres server, or a
  CloudNativePG `Cluster`.
- Secrets: Corral's session and secrets keys and ContainerSSH's host key, generated once by
  `helm install` unless you name existing Secrets, which GitOps tools need (see
  [GitOps](#gitops-argo-cd-flux)).
- A NetworkPolicy that lets only ContainerSSH reach Corral's webhook port.

In the sandbox namespace (`<release>-sandboxes` by default, created by the chart unless
`sandboxes.namespace.create` is false):

- A Role for Corral: Sandboxes, Secrets and PersistentVolumeClaims, reading Pods and Events, and
  exec into Pods for the idle check. A Role for ContainerSSH: reading Pods and exec into them.
- The ServiceAccount sandboxes run as, with no token mounted.

Neither Corral nor ContainerSSH has rights outside the sandbox namespace, and users never get
cluster access. The chart does not install agent-sandbox or the CloudNativePG operator.

## Prerequisites

- Kubernetes 1.35 or later: sandboxes mount the ContainerSSH agent and the Corral overlay as
  image volumes.
- agent-sandbox v1.0.11. `helm install` fails until it is installed:

  ```sh
  kubectl apply --server-side -f https://github.com/kubernetes-sigs/agent-sandbox/releases/download/v1.0.11/sandbox.yaml
  ```

- A default StorageClass, or `database.sqlite.storageClass` and `sandboxes.storageClass`.
- A load balancer for the ContainerSSH Service (MetalLB, a cloud provider), or
  `containerssh.service.type: NodePort`.
- A network plugin that enforces NetworkPolicy. Without one, sandboxes can reach Corral's
  unauthenticated webhook port, which wakes any sandbox it is asked about; set
  `networkPolicy.enabled: false` only where something else keeps them apart.
- For `database.kind: cnpg`, the [CloudNativePG](https://cloudnative-pg.io) operator, 1.23 or
  later for `database.cnpg.enablePDB: false`.

## Install

The chart is published as an OCI artifact. Each build of `main` is version
`<chart version>-main.<CI run number>`, so a newer build always sorts higher, and runs the Corral
image `ghcr.io/adamaq01/corral:sha-<commit>` from the same commit. To follow `main`, use the
version range `>=0.0.0-0` (it includes these prerelease versions); `--devel` does the same for a
plain `helm install` without `--version`.

1. Write a values file with at least the two required values:

   ```yaml
   baseUrl: https://corral.example.com
   ssh:
     publicHost: ssh.example.com
   ```

   `../examples/` holds complete values files: `minimal.yaml`, `homelab.yaml` (OIDC,
   CloudNativePG, OpenBao, Gateway API, MetalLB) and `postgres-ingress.yaml` (an existing
   Postgres, a Vault token, an Ingress, a NodePort).

2. Install:

   ```sh
   helm install corral oci://ghcr.io/adamaq01/charts/corral --version <version> \
     --namespace corral --create-namespace --values values.yaml
   ```

3. Create the first admin. The password is prompted for twice, at least 12 characters:

   ```sh
   kubectl --namespace corral exec -it deploy/corral -- corral-server local-account create <name> --admin
   ```

4. Open `baseUrl`, sign in, add an SSH key and create a sandbox, then connect with the command
   its page shows. Users see this host key fingerprint on first connect:

   ```sh
   ssh-keyscan -p 48291 ssh.example.com | ssh-keygen -lf -
   ```

## Values

`values.yaml` documents every value, and `values.schema.json` checks them: a misspelled key, a
wrong type or a missing required combination fails `helm install`, `helm upgrade` and `helm lint`
with the path of the offending value. The required combinations:

- `baseUrl` and `ssh.publicHost` always.
- At least one login method: with `identity.local.enabled: false`, `identity.oidc.enabled` must
  be true.
- With OIDC on: `issuer`, `clientId` and `clientSecret`. Register `<baseUrl>/auth/oidc/callback`
  as the redirect URI. `identity.oidc.opkssh.enabled` also lets OIDC users reach SSH with
  `opkssh login`, through the issuer's client `identity.oidc.opkssh.clientId`, for certificates
  that last `identity.oidc.opkssh.policy`; [docs/opkssh.md](../../../docs/opkssh.md) has what the
  issuer needs. The chart then mounts empty directories at `/etc/opk`, `/var/cache` and
  `/var/log` for opkssh's files.
- With `secretStore.kind: vault`: `address`, and a `role` for Kubernetes auth or a `token` for
  token auth. Kubernetes auth logs in with the ServiceAccount named after the release
  (`corral` for a release named corral), in the release namespace. Its policy needs `create`,
  `read`, `update`, `delete` and `list` on `<mount>/data/<pathPrefix>/*` and
  `<mount>/metadata/<pathPrefix>/*`.
- With `database.kind: postgres`: `database.postgres.url`.
- Either `ingress` or `httpRoute`, not both.

A secret value (`identity.oidc.clientSecret`, `secretStore.vault.auth.token`,
`database.postgres.url`) is either `existingSecret` and `key`, naming a Secret in the release
namespace, or `value`, which the chart stores in a Secret of its own. `value` puts the secret in
your values file and in the release history; prefer `existingSecret`.

`keys.existingSecret` names a Secret with `CORRAL_SESSION_KEY` and `CORRAL_SECRETS_KEY`, such as
one made from `corral-server keys generate`. `containerssh.hostKey.existingSecret` names a Secret
whose `host_key` is an SSH host private key, such as the `containerssh_host_key` of a Compose
install, so clients keep trusting the same host.

## GitOps (Argo CD, Flux)

Argo CD renders the chart with `helm template` and applies the result, so there is no Helm
release and Helm's `lookup` finds nothing in the cluster. The chart generates the keys and host
key once and reads them back with `lookup` on every upgrade; under Argo CD it would make new ones
at every render, leaving stored secrets unreadable and every client facing a new host key.
Rendered without a cluster, the chart therefore fails until `keys.existingSecret` and
`containerssh.hostKey.existingSecret` name existing Secrets. A `helm template` preview needs them
too, or use `helm install --dry-run=server` against the cluster instead.

Create the two Secrets once in the release namespace, outside the chart, so that neither a sync
nor deleting the Application touches them. Keep their values in a secret store, since losing
`CORRAL_SECRETS_KEY` makes every stored secret unreadable. With the
[External Secrets Operator](https://external-secrets.io), store `CORRAL_SESSION_KEY` and
`CORRAL_SECRETS_KEY` from `docker run --rm ghcr.io/adamaq01/corral keys generate`, and a
`host_key` from `ssh-keygen -q -t ed25519 -N '' -C containerssh -f host_key`, under one key of
the store, then sync them into the release namespace:

```yaml
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata:
  name: corral-keys
  namespace: corral
spec:
  secretStoreRef: { kind: ClusterSecretStore, name: openbao }
  data:
    - secretKey: CORRAL_SESSION_KEY
      remoteRef: { key: corral, property: CORRAL_SESSION_KEY }
    - secretKey: CORRAL_SECRETS_KEY
      remoteRef: { key: corral, property: CORRAL_SECRETS_KEY }
---
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata:
  name: corral-host-key
  namespace: corral
spec:
  secretStoreRef: { kind: ClusterSecretStore, name: openbao }
  data:
    - secretKey: host_key
      remoteRef: { key: corral, property: host_key }
```

```yaml
keys:
  existingSecret: corral-keys
containerssh:
  hostKey:
    existingSecret: corral-host-key
```

Without a secret store, save the output of `keys generate` to a file and make the same Secrets
with `kubectl create secret generic`: `--from-env-file` for the keys, `--from-file=host_key` for
the host key.

The rest of the chart works under Argo CD as under Helm, including `keepOnUninstall` (see
[Uninstall](#uninstall)). The agent-sandbox check runs only against a live cluster, so Argo CD
does not make it.

Flux's helm-controller runs Helm's own install and upgrade against the cluster, so `lookup` reads
the generated Secrets back as with `helm install`, and existing Secrets are optional there.

## Upgrade

```sh
helm upgrade corral oci://ghcr.io/adamaq01/charts/corral --version <version> \
  --namespace corral --values values.yaml
```

- Generated keys and the host key are read back from the cluster and kept.
- The overlay is `sandboxes.overlayImage`, of the chart's appVersion unless its tag says
  otherwise, so each chart version brings its own. A sandbox mounts it from its next start from
  sleep on; a running sandbox keeps the one it started with.
- A change to Corral's or ContainerSSH's config restarts its pods. Corral runs one replica and
  is replaced, not rolled, so the web UI and new SSH logins pause while it restarts; open SSH
  sessions continue.
- Sandboxes keep running, and their homes are untouched.

## Uninstall

`helm uninstall corral --namespace corral` deletes Corral, ContainerSSH and their config. Then:

- The generated keys and host key Secrets always stay, since stored secrets and users'
  `known_hosts` depend on them. Delete them by hand to start over.
- By default the database volume (or CloudNativePG Cluster) and the chart-created sandbox
  namespace are deleted, and with the namespace every sandbox and home.
- With `keepOnUninstall: true`, they stay. Installing the chart again with the same release name
  and namespace picks them up: users, sandboxes and homes come back.
- With `sandboxes.namespace.create: false`, the namespace and the sandboxes in it stay; delete
  the sandboxes from the web UI before uninstalling if their homes should go.

Argo CD keeps the same resources when its Application is deleted, and never prunes them when a
values change drops them: they carry `argocd.argoproj.io/sync-options: Delete=false,Prune=false`
next to Helm's `helm.sh/resource-policy: keep`. Flux runs `helm uninstall`, which reads the
latter.

## Operating it

- Logs: `kubectl --namespace corral logs deploy/corral`. `logLevel` sets the level.
- A sandbox sleeps after `idle.timeoutSecs` with no SSH session and no Herdr terminal at work,
  and wakes on the next SSH login. A connection that only forwards ports (`ssh -N -L`) keeps a
  sandbox awake while a forwarded connection through it is open, such as a page holding a dev
  server's live-reload socket, and not while the tunnel carries nothing.
- A sandbox pod's init container runs as root, only the overlay's static `corral-init`, which
  writes the sandbox user's account files; the sandbox container runs as uid 1000. The sandbox
  namespace enforces the `baseline` Pod Security level, which allows both.
- ContainerSSH runs an SSH command (`ssh host <command>`) through the image's `/bin/sh`, so an
  image without `/bin/sh` takes interactive logins and sftp only.
- The `corral` command in a sandbox (`corral status`, `restart`, `stop`, `delete --confirm`)
  reaches Corral's API at its Service, `http://<release>.<namespace>.svc`, which the
  NetworkPolicy leaves open on the HTTP port.
- With `catalog.namedImages.enabled`, users can also name an image of their own, from
  `catalog.namedImages.registries` and at most `catalog.namedImages.maxSize` to download. Nodes
  pull private images with `sandboxes.imagePullSecrets`; Corral reads their sizes with the
  logins under `registries`. [docs/images.md](../../../docs/images.md) has both.
- A change to a `catalog.images` entry's `reference` or to a size reaches each sandbox made
  from it at its next start from sleep: its pod template is made again with the new image,
  requests and limits, its home kept. A running sandbox keeps what it has until then, and a
  sandbox whose image or size is no longer in the catalog keeps running as it was made. A
  size's `homeSize` applies to new sandboxes only: an existing home's claim keeps its size.
- Behind an Ingress controller or Gateway, set `trustForwardedFor: true` so password-login limits
  count each client, not the proxy.
- With `database.kind: cnpg`, the operator's PodDisruptionBudgets keep a node drain from evicting
  the primary, so a single instance blocks draining its node. `database.cnpg.enablePDB: false`
  removes them; a drain then stops the database until the instance starts again elsewhere.
- `containerssh.service.externalTrafficPolicy: Local` keeps clients' addresses, so Corral's
  lockout after 10 failed SSH connections in 5 minutes applies per client (per /64 for IPv6).

## Lockout behind a tunnel

Corral locks out a source address after 10 failed SSH connections in 5 minutes. ContainerSSH
does not speak the PROXY protocol, so a connection relayed by a TCP tunnel or proxy reaches it
from the relay's address. Every client behind the relay then shares one lockout, and 10
failures by anyone lock everyone out for 5 minutes.

`containerssh.lockoutExemptNetworks` lists the networks such relays connect from, in CIDR
notation. Connections from them are never locked out; connections from anywhere else still
are. Set it when public SSH reaches ContainerSSH through a tunnel such as rathole, frp or an SSH
reverse tunnel, or through a TCP proxy, and leave it empty when clients connect directly.

Nothing replaces the lockout for exempt connections. ContainerSSH accepts public keys and
certificates only, which cannot be guessed, and tries at most 6 keys on each connection. A
lockout per account would let anyone lock a user out by failing as them, so there is none.

For example, a rathole server on a public host forwards port 48291 to a rathole client in the
cluster, which connects to the ContainerSSH Service. The server's `server.toml`:

```toml
[server]
bind_addr = "0.0.0.0:2333"

[server.services.corral-ssh]
token = "<a long random secret>"
bind_addr = "0.0.0.0:48291"
```

The client's `client.toml`, run in a pod in the cluster:

```toml
[client]
remote_addr = "tunnel.example.com:2333"

[client.services.corral-ssh]
token = "<the same secret>"
local_addr = "corral-containerssh.corral.svc.cluster.local:48291"
```

The client connects from its pod's address, which changes when the pod is replaced, so exempt
the pod network it runs in, here k3s's default:

```yaml
ssh:
  publicHost: tunnel.example.com
containerssh:
  lockoutExemptNetworks: [10.42.0.0/16]
```

LAN clients reaching the MetalLB address with `externalTrafficPolicy: Local` keep their own
addresses and stay locked out individually, as long as the LAN is outside the exempt networks.
Every pod in an exempt network is exempt too, sandboxes included; a narrower network, such as
the pod CIDR of the one node the client is pinned to, exempts less.

## Tests

- `deploy/helm/tools.sh <dir>` installs the pinned helm, kubeconform and helm-unittest.
- `deploy/helm/test.sh` lints the chart, checks the schema against `../schema-tests/` and the
  examples, checks that `helm template` refuses to generate keys, validates the rendered
  manifests against the Kubernetes schemas, checks the rendered config with
  `corral-server config check`, and runs the unit tests in `tests/`.
- `deploy/helm/e2e.sh` installs the chart into a kind cluster with agent-sandbox and walks
  through sign-in, SSH, port forwarding, idle sleep and wake, upgrade and uninstall.
