# Corral Helm chart

Runs Corral on Kubernetes, with [agent-sandbox](https://github.com/kubernetes-sigs/agent-sandbox)
as the sandbox engine and [ContainerSSH](https://containerssh.io) as the SSH front door. Each
sandbox is an agent-sandbox `Sandbox` whose home is a PersistentVolumeClaim. Users reach a sandbox
with any SSH client:

```
ssh -p 48291 <handle>+<sandbox>@<ssh.publicHost>
```

## What it installs

In the release namespace:

- Corral: a Deployment of one replica, a Service for the web UI and API (port 8080), a ClusterIP
  Service `<release>-webhooks` for ContainerSSH's webhooks (port 8081), its config rendered from
  the values, and an optional Ingress or Gateway API HTTPRoute for `baseUrl`'s host.
- ContainerSSH with its Kubernetes backend: public keys only, port forwarding on, and a webhook
  timeout 10 seconds above the wake deadline. Its Service is a LoadBalancer by default.
- The database: SQLite on a PersistentVolumeClaim by default, an existing Postgres server, or a
  CloudNativePG `Cluster`.
- Secrets: Corral's session and secrets keys and ContainerSSH's host key, generated once unless
  you name existing Secrets.
- A NetworkPolicy that lets only ContainerSSH reach Corral's webhook port.

In the sandbox namespace (`<release>-sandboxes` by default, created by the chart unless
`sandboxes.namespace.create` is false):

- A Role for Corral: Sandboxes, Secrets and PersistentVolumeClaims, reading Pods, and exec into
  Pods for the idle check. A Role for ContainerSSH: reading Pods and exec into them.
- The ServiceAccount sandboxes run as, with no token mounted.

Neither Corral nor ContainerSSH has rights outside the sandbox namespace, and users never get
cluster access. The chart does not install agent-sandbox or the CloudNativePG operator.

## Prerequisites

- Kubernetes 1.35 or later: sandboxes mount the ContainerSSH agent as an image volume.
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
- For `database.kind: cnpg`, the [CloudNativePG](https://cloudnative-pg.io) operator.

## Install

The chart is published as an OCI artifact. Each build of `main` is version
`0.1.0-sha-<commit>` and runs the Corral image `ghcr.io/adamaq01/corral:sha-<commit>` from the
same commit.

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
  as the redirect URI.
- With `secretStore.kind: vault`: `address`, and a `role` for Kubernetes auth or a `token` for
  token auth. Kubernetes auth logs in with the ServiceAccount named after the release
  (`corral` for a release named corral), in the release namespace.
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

## Upgrade

```sh
helm upgrade corral oci://ghcr.io/adamaq01/charts/corral --version <version> \
  --namespace corral --values values.yaml
```

- Generated keys and the host key are read back from the cluster and kept.
- A change to Corral's or ContainerSSH's config restarts its pods. Corral runs one replica and
  is replaced, not rolled, so the web UI and new SSH logins pause while it restarts; open SSH
  sessions continue.
- Sandboxes keep running, and their homes are untouched.

## Uninstall

`helm uninstall corral --namespace corral` deletes Corral, ContainerSSH and their config. Then:

- The generated keys and host key Secrets always stay, since stored credentials and users'
  `known_hosts` depend on them. Delete them by hand to start over.
- By default the database volume (or CloudNativePG Cluster) and the chart-created sandbox
  namespace are deleted, and with the namespace every sandbox and home.
- With `keepOnUninstall: true`, they stay. Installing the chart again with the same release name
  and namespace picks them up: users, sandboxes and homes come back.
- With `sandboxes.namespace.create: false`, the namespace and the sandboxes in it stay; delete
  the sandboxes from the web UI before uninstalling if their homes should go.

## Operating it

- Logs: `kubectl --namespace corral logs deploy/corral`. `logLevel` sets the level.
- A sandbox sleeps after `idle.timeoutSecs` with no SSH session and no Herdr terminal at work,
  and wakes on the next SSH login. A connection that only forwards ports (`ssh -N -L`) does not
  keep a sandbox awake.
- Behind an Ingress controller or Gateway, set `trustForwardedFor: true` so password-login limits
  count each client, not the proxy.
- `containerssh.service.externalTrafficPolicy: Local` keeps clients' addresses, so Corral's
  lockout after 10 failed SSH logins in 5 minutes applies per client.

## Tests

- `deploy/helm/tools.sh <dir>` installs the pinned helm, kubeconform and helm-unittest.
- `deploy/helm/test.sh` lints the chart, checks the schema against `../schema-tests/` and the
  examples, validates the rendered manifests against the Kubernetes schemas, checks the rendered
  config with `corral-server config check`, and runs the unit tests in `tests/`.
- `deploy/helm/e2e.sh` installs the chart into a kind cluster with agent-sandbox and walks
  through sign-in, SSH, port forwarding, idle sleep and wake, upgrade and uninstall.
