{{/* Names */}}
{{- define "chakramcp.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "chakramcp.fullname" -}}
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

{{- define "chakramcp.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* Labels. The server's pods are component "relay", the name its Compose
service has, which the dashboards and alert rules key on. */}}
{{- define "chakramcp.labels" -}}
helm.sh/chart: {{ include "chakramcp.chart" . }}
{{ include "chakramcp.selectorLabels" . }}
app.kubernetes.io/version: {{ include "chakramcp.imageTag" . | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "chakramcp.selectorLabels" -}}
app.kubernetes.io/name: {{ include "chakramcp.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "chakramcp.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "chakramcp.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "chakramcp.imageTag" -}}
{{- .Values.image.tag | default .Chart.AppVersion }}
{{- end }}

{{/* Secrets */}}
{{- define "chakramcp.appSecretName" -}}
{{- .Values.secrets.existingSecret | default (printf "%s-secrets" (include "chakramcp.fullname" .)) }}
{{- end }}

{{- define "chakramcp.postgresql.fullname" -}}
{{- printf "%s-postgresql" (include "chakramcp.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "chakramcp.postgresql.secretName" -}}
{{- .Values.postgresql.auth.existingSecret | default (include "chakramcp.postgresql.fullname" .) }}
{{- end }}

{{- define "chakramcp.redis.fullname" -}}
{{- printf "%s-redis" (include "chakramcp.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* The chart-managed Secret for external connection URLs given as values. */}}
{{- define "chakramcp.connectionsSecretName" -}}
{{- printf "%s-connections" (include "chakramcp.fullname" .) }}
{{- end }}

{{- define "chakramcp.externalDatabaseFromValues" -}}
{{- if and (not .Values.postgresql.enabled) (not .Values.externalDatabase.existingSecret) .Values.externalDatabase.url }}true{{ end }}
{{- end }}

{{- define "chakramcp.externalRedisFromValues" -}}
{{- if and (not .Values.redis.enabled) .Values.externalRedis.url }}true{{ end }}
{{- end }}

{{/* Public URLs: the explicit value, else the Ingress host (https when the
host has TLS). Empty means the server's own default. */}}
{{- define "chakramcp.publicUrl" -}}
{{- $root := index . 0 }}{{- $explicit := index . 1 }}{{- $host := index . 2 }}
{{- if $explicit }}
{{- $explicit }}
{{- else if and $root.Values.ingress.enabled $host }}
{{- $tls := false }}
{{- range $root.Values.ingress.tls }}{{ if has $host .hosts }}{{ $tls = true }}{{ end }}{{ end }}
{{- printf "%s://%s" (ternary "https" "http" $tls) $host }}
{{- end }}
{{- end }}

{{/* A random secret that survives upgrades: the value already in the named
Secret when there is one, else a new one. */}}
{{- define "chakramcp.keptRandom" -}}
{{- $root := index . 0 }}{{- $name := index . 1 }}{{- $key := index . 2 }}{{- $length := index . 3 }}
{{- $existing := lookup "v1" "Secret" $root.Release.Namespace $name }}
{{- if and $existing (hasKey (default dict $existing.data) $key) }}
{{- index $existing.data $key }}
{{- else }}
{{- randAlphaNum $length | b64enc }}
{{- end }}
{{- end }}

{{/* Readable failures for combinations the schema can only describe
tersely. Included by the Deployment. */}}
{{- define "chakramcp.validate" -}}
{{- if and (not .Values.postgresql.enabled) (not .Values.externalDatabase.url) (not .Values.externalDatabase.existingSecret) }}
{{- fail "postgresql.enabled=false needs externalDatabase.url or externalDatabase.existingSecret" }}
{{- end }}
{{- if and .Values.ingress.enabled (or (not .Values.ingress.hosts.app) (not .Values.ingress.hosts.relay)) }}
{{- fail "ingress.enabled=true needs both ingress.hosts.app and ingress.hosts.relay" }}
{{- end }}
{{- end }}
