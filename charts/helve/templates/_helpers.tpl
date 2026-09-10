{{/*
Standard chart name / fullname / labels, following the same pattern `helm
create` scaffolds.
*/}}
{{- define "helve.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "helve.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "helve.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "helve.labels" -}}
helm.sh/chart: {{ include "helve.chart" . }}
{{ include "helve.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "helve.selectorLabels" -}}
app.kubernetes.io/name: {{ include "helve.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "helve.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "helve.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "helve.image" -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}

{{/*
Effective per-deployment-proxy base domain: proxy.baseDomain if set, else
"proxy.<host>" — matching how backend/src/main.rs describes the same
default relationship in its own --proxy-base-domain help text.
*/}}
{{- define "helve.proxyBaseDomain" -}}
{{- if .Values.proxy.baseDomain -}}
{{- .Values.proxy.baseDomain -}}
{{- else -}}
{{- printf "proxy.%s" .Values.host -}}
{{- end -}}
{{- end -}}

{{/*
The scheme APP_ORIGIN and the Ingress are served over. Only "https" turns
on Secure session cookies (backend/src/state.rs::cookies_secure) — so this
must track whatever the Ingress actually terminates.
*/}}
{{- define "helve.scheme" -}}
{{- if and .Values.ingress.enabled .Values.ingress.tls.enabled -}}
{{- "https" -}}
{{- else -}}
{{- "http" -}}
{{- end -}}
{{- end -}}

{{- define "helve.appOrigin" -}}
{{- printf "%s://%s" (include "helve.scheme" .) .Values.host -}}
{{- end -}}

{{/*
The Cluster name for the optional bundled CloudNativePG Postgres, and the
Secret CNPG auto-generates for it (pattern "<Cluster name>-app").
*/}}
{{- define "helve.dbClusterName" -}}
{{- printf "%s-db" (include "helve.fullname" .) -}}
{{- end -}}

{{- define "helve.dbSecretName" -}}
{{- printf "%s-app" (include "helve.dbClusterName" .) -}}
{{- end -}}

{{- define "helve.adminBootstrapSecretName" -}}
{{- printf "%s-admin-bootstrap" (include "helve.fullname" .) -}}
{{- end -}}

{{/*
A chart shipped to strangers should refuse to render something broken or
quietly insecure rather than half-work. Included once, from a template
that's always rendered (deployment.yaml), so `fail` actually halts the
release rather than being dead code sitting in an unused named template.
*/}}
{{- define "helve.validate" -}}

{{- if not .Values.host -}}
{{ fail "host is required — set the public hostname Helve is served from, e.g. --set host=helve.example.com" }}
{{- end -}}

{{- if and (not .Values.proxy.separateOrigins) (not .Values.proxy.allowSameOriginProxy) -}}
{{ fail "proxy.separateOrigins=false serves every proxied deployment (JupyterLab, RStudio, ...) from a path on Helve's own origin, letting that deployment's JavaScript call Helve's API as whoever is browsing it. Set proxy.allowSameOriginProxy=true to accept that risk (e.g. for local development), or leave proxy.separateOrigins at its default of true." }}
{{- end -}}

{{- if and (not .Values.database.existingSecret) (not .Values.database.deploy.enabled) -}}
{{ fail "no database configured — either set database.existingSecret to a Secret holding a Postgres connection string (key: database.existingSecretKey, default \"uri\"), or set database.deploy.enabled=true to deploy a single-replica evaluation Postgres (requires the CloudNativePG operator; not for anything you'd be upset to lose)." }}
{{- end -}}

{{- if and (eq .Values.homeDrives.mode "hostPath") (not .Values.homeDrives.hostPath.basePath) -}}
{{ fail "homeDrives.mode=hostPath requires homeDrives.hostPath.basePath — the path every node already has the same shared filesystem mounted at." }}
{{- end -}}

{{- if eq .Values.homeDrives.mode "pvc" -}}
{{- if not .Values.homeDrives.pvc.storageClassName -}}
{{ fail "homeDrives.mode=pvc requires homeDrives.pvc.storageClassName (a ReadWriteMany-capable StorageClass)." }}
{{- end -}}
{{- if not .Values.homeDrives.pvc.size -}}
{{ fail "homeDrives.mode=pvc requires homeDrives.pvc.size, e.g. \"10Gi\"." }}
{{- end -}}
{{- end -}}

{{- if and .Values.ingress.enabled .Values.ingress.tls.enabled (eq .Values.ingress.tls.mode "certManager") (not .Values.ingress.tls.issuerRef.name) -}}
{{ fail "ingress.tls.mode=certManager requires ingress.tls.issuerRef.name (the cert-manager Issuer/ClusterIssuer to request the certificate from)." }}
{{- end -}}

{{- if and .Values.proxy.separateOrigins .Values.ingress.enabled .Values.ingress.tls.enabled (eq .Values.ingress.tls.mode "certManager") -}}
{{- $host := .Values.host -}}
{{- $base := include "helve.proxyBaseDomain" . -}}
{{- $hostSuffix := printf ".%s" $host -}}
{{- if hasSuffix $hostSuffix $base -}}
{{- $prefix := trimSuffix $hostSuffix $base -}}
{{- if contains "." $prefix -}}
{{ fail (printf "proxy.baseDomain %q is more than one label deeper than host %q. A wildcard certificate (\"*.%s\") only ever covers one label in front of it, matching ProxyOrigin::deployment_for_host in the backend — set proxy.baseDomain to exactly one label plus host, e.g. \"proxy.%s\"." $base $host $base $host) }}
{{- end -}}
{{- end -}}
{{- end -}}

{{- end -}}
