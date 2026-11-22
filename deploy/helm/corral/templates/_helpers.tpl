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

{{- define "corral.containersshName" -}}
{{- printf "%s-containerssh" (include "corral.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "corral.webhooksName" -}}
{{- printf "%s-webhooks" (include "corral.fullname" .) | trunc 63 | trimSuffix "-" }}
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
{{- .Values.containerssh.hostKey.existingSecret | default (printf "%s-host-key" (include "corral.containersshName" .)) }}
{{- end }}

{{- define "corral.baseHost" -}}
{{- (urlParse .Values.baseUrl).hostname }}
{{- end }}

{{- define "corral.sshServicePort" -}}
{{- .Values.containerssh.service.port | default .Values.ssh.publicPort | int }}
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
{{- toJson $env }}
{{- end }}

{{- define "corral.keepAnnotation" -}}
{{- if .Values.keepOnUninstall }}
helm.sh/resource-policy: keep
{{- end }}
{{- end }}

{{/*
"true" when OIDC and its opkssh logins are on, else empty.
*/}}
{{- define "corral.opkssh" -}}
{{- if and .Values.identity.oidc.enabled .Values.identity.oidc.opkssh.enabled }}true{{ end }}
{{- end }}
