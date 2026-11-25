{{- define "corral.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "corral.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "corral.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/name: {{ include "corral.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "corral.selectorLabels" -}}
app.kubernetes.io/name: {{ include "corral.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "corral.gateName" -}}
{{- printf "%s-gate" (include "corral.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "corral.internalName" -}}
{{- printf "%s-internal" (include "corral.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "corral.sandboxNamespace" -}}
{{- .Values.sandboxes.namespace.name | default (printf "%s-sandboxes" (include "corral.fullname" .)) }}
{{- end }}

{{- define "corral.sandboxServiceAccount" -}}
{{- printf "%s-sandbox" (include "corral.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "corral.keysSecret" -}}
{{- .Values.keys.existingSecret | default (printf "%s-keys" (include "corral.fullname" .)) }}
{{- end }}

{{- define "corral.hostKeySecret" -}}
{{- .Values.ssh.hostKey.existingSecret | default (printf "%s-ssh-host-key" (include "corral.fullname" .)) }}
{{- end }}

{{/*
"true" when the chart is rendered against a cluster, else empty: lookup finds nothing under
helm template, helm lint or Argo CD.
*/}}
{{- define "corral.live" -}}
{{- if lookup "v1" "Namespace" "" "kube-system" }}true{{ end }}
{{- end }}

{{- define "corral.baseHost" -}}
{{- (urlParse .Values.baseUrl).hostname }}
{{- end }}

{{- define "corral.sshServicePort" -}}
{{- .Values.gate.service.port | default .Values.ssh.publicPort | int }}
{{- end }}

{{/*
The environment variables that carry a secret, each as name and either a value the chart stores
in its own Secret or a reference to an existing Secret.
*/}}
{{- define "corral.secretEnv" -}}
{{- $env := list }}
{{- with .Values.identity.oidc }}{{ if .enabled }}
{{- $env = append $env (dict "name" "CORRAL_OIDC_CLIENT_SECRET" "ref" .clientSecret) }}
{{- end }}{{ end }}
{{- with .Values.secretStore }}{{ if and (eq .kind "vault") (eq .vault.auth.kind "token") }}
{{- $env = append $env (dict "name" "CORRAL_VAULT_TOKEN" "ref" .vault.auth.token) }}
{{- end }}{{ end }}
{{- if eq .Values.database.kind "postgres" }}
{{- $env = append $env (dict "name" "DATABASE_URL" "ref" .Values.database.postgres.url) }}
{{- end }}
{{- range $index, $registry := .Values.registries }}{{ if $registry.username }}
{{- $env = append $env (dict "name" (printf "CORRAL_REGISTRY_%d_PASSWORD" $index) "ref" $registry.password) }}
{{- end }}{{ end }}
{{- toJson $env }}
{{- end }}

{{/*
Keeps a resource when Helm (uninstall, upgrade, rollback) or Argo CD (deleting the Application,
pruning) would delete it.
*/}}
{{- define "corral.keep" -}}
helm.sh/resource-policy: keep
argocd.argoproj.io/sync-options: Delete=false,Prune=false
{{- end }}

{{- define "corral.keepOnUninstall" -}}
{{- if .Values.keepOnUninstall }}{{ include "corral.keep" . }}{{ end }}
{{- end }}

{{/*
"true" when OIDC and its opkssh logins are on, else empty.
*/}}
{{- define "corral.opkssh" -}}
{{- if and .Values.identity.oidc.enabled .Values.identity.oidc.opkssh.enabled }}true{{ end }}
{{- end }}
