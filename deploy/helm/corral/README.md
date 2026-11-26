# Corral Helm chart

Runs Corral on Kubernetes, with [agent-sandbox](https://github.com/kubernetes-sigs/agent-sandbox)
as the sandbox engine and the gate (`corral-server gate`, from the same image) as the SSH front
door. Each sandbox is an agent-sandbox `Sandbox` whose home is a PersistentVolumeClaim, from any
image: the Corral overlay, mounted into every sandbox, brings the sandbox user, sudo, the login
shell, OpenSSH, Herdr and the `corral` command ([docs/images.md](../../../docs/images.md)). Users
reach a sandbox with any SSH client:

```
ssh -p 48291 <sandbox>.<handle>@<ssh.publicHost>
```

A bare `<handle>` reaches the user's default sandbox. A sandbox name the user has not used yet
makes that sandbox, unless they already have as many sandboxes as their limit allows: on a
terminal an installer asks for its image, size and setup, starting from their **Account**
defaults, then opens its shell; `corral-new` makes it from a JSON spec on stdin; any other
command makes it from their Account defaults ([docs/lobby.md](../../../docs/lobby.md)). A
connection that reaches no sandbox, such as one past that limit, to a sandbox scheduled for
deletion, or by a disabled user, is told why at once: a command prints the reason on its standard
error and exits 1, a terminal shows it, and the connection ends with it.

The gate waits for the first thing a connection asks (a shell, a command, sftp or a forward)
before it wakes or makes the sandbox, and counts the seconds on a terminal while it wakes. Each
connection then runs the overlay's own OpenSSH in the sandbox's pod, as the sandbox user, through
a pod exec, so sessions behave exactly as OpenSSH: the login shell, variables, signals, exit
status, sftp, port and socket forwards both ways, and agent forwarding.

## What it installs

In the release namespace:

- Corral: a Deployment of one replica, a Service for the web UI and API (port 8080), a ClusterIP
  Service `<release>-internal` for the internal API the gate asks (port 8081), its config
  rendered from the values, and an optional Ingress or Gateway API HTTPRoute for `baseUrl`'s
  host.
- The gate, `<release>-gate`: a Deployment of `gate.replicas` pods of the Corral image running
  `corral-server gate` with the same config, and its Service, a LoadBalancer by default. It holds
  no state, so a Corral restart leaves open SSH sessions up, and only a change to what it reads
  (the `ssh` values) restarts it.
- The database: SQLite on a PersistentVolumeClaim by default, an existing Postgres server, or a
  CloudNativePG `Cluster`.
- Secrets: Corral's session and secrets keys, the secret Corral and the gate share, and the SSH
  host key, generated once by `helm install` unless you name existing Secrets, which GitOps tools
  need (see [GitOps](#gitops-argo-cd-flux)).
- A NetworkPolicy that lets only the gate reach Corral's internal port.

In the sandbox namespace (`<release>-sandboxes` by default, created by the chart unless
`sandboxes.namespace.create` is false):

- A Role for Corral: Sandboxes, Secrets and PersistentVolumeClaims, reading Pods and Events. A
  Role for the gate: exec into Pods, nothing more.
- The ServiceAccount sandboxes run as, with no token mounted.

Corral then makes, for each sandbox, a `Sandbox`, its home claim `home-<id>` and the Secrets
`<id>-env` and `<id>-home`. The id is `<handle>-<sandbox>-<6 hex>` (`sbx-<12 hex>` for an older
sandbox), and each of these objects and the pod carry the labels and annotations
`corral.adamaq01.fr/owner` (the handle), `corral.adamaq01.fr/user-id` and
`corral.adamaq01.fr/sandbox` (the name), so a released volume tells whose it was. The pod runs
with `supplementalGroupsPolicy: Strict`, which needs a container runtime that supports it, such
as containerd 2.0 or later or CRI-O 1.31 or later: the sandbox user gets only its own group, not
the groups the image's `/etc/group` lists for uid 1000.

Neither Corral nor the gate has rights outside the sandbox namespace, and users never get
cluster access. The chart does not install agent-sandbox or the CloudNativePG operator.

## Prerequisites

- Kubernetes 1.35 or later: sandboxes mount the Corral overlay as an image volume.
- agent-sandbox v1.0.11. `helm install` fails until it is installed:

  ```sh
  kubectl apply --server-side -f https://github.com/kubernetes-sigs/agent-sandbox/releases/download/v1.0.11/sandbox.yaml
  ```

- A default StorageClass, or `database.sqlite.storageClass` and `sandboxes.storageClass`.
- A load balancer for the gate's Service (MetalLB, a cloud provider), or
  `gate.service.type: NodePort`.
- A network plugin that enforces NetworkPolicy. Without one, sandboxes can reach Corral's
  internal port; it answers only requests carrying the gate's secret, but set
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
- With `catalog.defaults`: both `image` and `size`.
- Either `ingress` or `httpRoute`, not both.

A secret value (`identity.oidc.clientSecret`, `secretStore.vault.auth.token`,
`database.postgres.url`, a registry's `password`) is either `existingSecret` and `key`, naming a
Secret in the release namespace, or `value`, which the chart stores in a Secret of its own.
`value` puts the secret in your values file and in the release history; prefer `existingSecret`.

`keys.existingSecret` names a Secret with `CORRAL_SESSION_KEY`, `CORRAL_SECRETS_KEY` and
`CORRAL_GATE_SECRET`, such as one made from `corral-server keys generate`.
`ssh.hostKey.existingSecret` names a Secret whose `host_key` is an SSH host private key, in
OpenSSH or PEM form, such as the `ssh_host_key` of a Compose install, so clients keep trusting the
same host.

The admin manages the catalog, the limits and registry logins in the web UI, under **Settings**.
What `catalog`, `limits` and `registries` set is shown there locked and changes with the values
alone. In Settings the admin adds more images, sizes and registries, and sets each limit the
values leave empty. A registry added there names the environment variable that holds its
password, and `registryPasswords` sets that variable in Corral's pod, from a secret value:

```yaml
registryPasswords:
  CORRAL_QUAY_TOKEN:
    existingSecret: corral-registries
    key: quay
```

## GitOps (Argo CD, Flux)

Argo CD renders the chart with `helm template` and applies the result, so there is no Helm
release and Helm's `lookup` finds nothing in the cluster. The chart generates the keys and host
key once and reads them back with `lookup` on every upgrade; under Argo CD it would make new ones
at every render, leaving stored secrets unreadable and every client facing a new host key.
Rendered without a cluster, the chart therefore fails until `keys.existingSecret` and
`ssh.hostKey.existingSecret` name existing Secrets. A `helm template` preview needs them
too, or use `helm install --dry-run=server` against the cluster instead.

Create the two Secrets once in the release namespace, outside the chart, so that neither a sync
nor deleting the Application touches them. Keep their values in a secret store, since losing
`CORRAL_SECRETS_KEY` makes every stored secret unreadable. With the
[External Secrets Operator](https://external-secrets.io), store `CORRAL_SESSION_KEY`,
`CORRAL_SECRETS_KEY` and `CORRAL_GATE_SECRET` from
`docker run --rm ghcr.io/adamaq01/corral keys generate`, and a `host_key` from
`ssh-keygen -q -t ed25519 -N '' -C corral -f host_key`, under one key of the store, then sync
them into the release namespace:

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
    - secretKey: CORRAL_GATE_SECRET
      remoteRef: { key: corral, property: CORRAL_GATE_SECRET }
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
ssh:
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
- A sandbox running from before the gate, or without the overlay, takes no SSH session until its
  next start: restart it from its page, and the start makes its pod from the current template,
  its home kept.
- A change to Corral's config restarts its pod. Corral runs one replica and is replaced, not
  rolled, so the web UI and new SSH logins pause while it restarts; open SSH sessions continue,
  since the gate carries them. A change to the `ssh` values rolls the gate's pods, which ends the
  sessions they carry.
- Sandboxes keep running, and their homes are untouched.

## Uninstall

`helm uninstall corral --namespace corral` deletes Corral, the gate and their config. Then:

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

- Logs: `kubectl --namespace corral logs deploy/corral` and `deploy/corral-gate`. `logLevel`
  sets the level of both.
- A sandbox sleeps after `idle.timeoutSecs` with no work, and wakes on the next SSH login. Work
  is what a person does through SSH (typing, commands, file transfers, traffic through a forward)
  or the sandbox's own processor and network use, which it reports to Corral every
  `idle.reportIntervalSecs`; a report is work past `idle.busyCpuPercent` of one core or
  `idle.busyNetworkBytesPerSec`. An open but untouched connection, a forward with no traffic, and
  Herdr staying connected do not keep it awake. While it sleeps the gate answers Herdr for it,
  with no terminals, and the first terminal opened there wakes it.
- Each sandbox's page sets its own idle timeout, up to `idle.maxTimeoutSecs`, and keeps it awake
  up to `idle.maxKeepAwakeSecs` ahead; `corral keep-awake 4h` inside does the same. A sandbox
  that reports nothing, such as one started before an upgrade to this version, stays awake until
  it is restarted, and Corral logs that it does.
- A sandbox pod's init container runs as root, only the overlay's static `corral-init`, which
  writes the sandbox user's account files; the sandbox container runs as uid 1000. The sandbox
  namespace enforces the `baseline` Pod Security level, which allows both.
- The `corral` command in a sandbox (`corral status`, `restart`, `stop`, `delete --confirm`)
  reaches Corral's API at its Service, `http://<release>.<namespace>.svc`, which the
  NetworkPolicy leaves open on the HTTP port.
- Users can also name an image of their own once the admin allows it in Settings or
  `catalog.namedImages.enabled` does, from the registries it lists and up to the download size it
  sets. Nodes pull private images with `sandboxes.imagePullSecrets`; Corral reads their sizes
  with the registry logins in Settings. [docs/images.md](../../../docs/images.md) has both.
- A change to a catalog image's reference or to a size, in the values or in Settings, reaches
  each sandbox made from it at its next start from sleep: its pod template is made again with
  the new image, requests and limits, its home kept. A running sandbox keeps what it has until
  then, and a sandbox whose image or size is no longer in the catalog keeps running as it was
  made. A size's home size applies to new sandboxes only: an existing home's claim keeps its
  size.
- Behind an Ingress controller or Gateway, set `trustForwardedFor: true` so password-login limits
  count each client, not the proxy.
- With `database.kind: cnpg`, the operator's PodDisruptionBudgets keep a node drain from evicting
  the primary, so a single instance blocks draining its node. `database.cnpg.enablePDB: false`
  removes them; a drain then stops the database until the instance starts again elsewhere.
- `gate.service.externalTrafficPolicy: Local` keeps clients' addresses, so Corral's lockout
  after 10 failed SSH connections in 5 minutes applies per client (per /64 for IPv6).
- The gate faces the internet. A connection has a minute to authenticate and 6 attempts, at most
  64 connections wait to authenticate at once per gate pod, and each connection holds at most 64
  channels and 16 remote forwards. Public keys and opkssh certificates are the only logins.

## Lockout behind a tunnel

Corral locks out a source address after 10 failed SSH connections in 5 minutes. The gate does
not speak the PROXY protocol, so a connection relayed by a TCP tunnel or proxy reaches it
from the relay's address. Every client behind the relay then shares one lockout, and 10
failures by anyone lock everyone out for 5 minutes.

`ssh.lockoutExemptNetworks` lists the networks such relays connect from, in CIDR notation.
Connections from them are never locked out; connections from anywhere else still are. Set it
when public SSH reaches the gate through a tunnel such as rathole, frp or an SSH
reverse tunnel, or through a TCP proxy, and leave it empty when clients connect directly.

Nothing replaces the lockout for exempt connections. The gate accepts public keys and
certificates only, which cannot be guessed, and tries at most 6 keys on each connection. A
lockout per account would let anyone lock a user out by failing as them, so there is none.

For example, a rathole server on a public host forwards port 48291 to a rathole client in the
cluster, which connects to the gate's Service. The server's `server.toml`:

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
local_addr = "corral-gate.corral.svc.cluster.local:48291"
```

The client connects from its pod's address, which changes when the pod is replaced, so exempt
the pod network it runs in, here k3s's default:

```yaml
ssh:
  publicHost: tunnel.example.com
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
  through sign-in, SSH, port forwarding, idle sleep and wake; `deploy/helm/e2e.sh all` adds
  upgrade, uninstall and reinstall.

`just chart` and `just e2e-kind` run them; [docs/testing.md](../../../docs/testing.md) has the
rest.
