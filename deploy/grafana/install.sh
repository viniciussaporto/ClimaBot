#!/usr/bin/env bash
# Provisions the ClimaBot Grafana dashboards into the monitoring stack's
# provisioning directory. Grafana picks up changes within 30 seconds; no
# restart needed. Safe to run again (deploy/install.sh runs it on updates).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
PROV=${GRAFANA_PROVISIONING_DIR:-/root/monitoring/grafana-provisioning}
DASH="$PROV/dashboards"

if [[ ! -d "$DASH" ]]; then
    echo "Grafana provisioning directory $DASH not found; skipping dashboards"
    exit 0
fi

install -d -m 0755 "$DASH/climabot" "$DASH/infrastructure"
install -m 0644 "$here/climabot.json" "$DASH/climabot/climabot.json"
install -m 0644 "$here/docker-host.json" "$DASH/infrastructure/docker-host.json"

cat >"$DASH/climabot.yml" <<'EOF'
# Managed by ClimaBot deploy/grafana/install.sh: edits here are overwritten.
apiVersion: 1
providers:
  - name: ClimaBot
    orgId: 1
    folder: ClimaBot
    type: file
    disableDeletion: false
    allowUiUpdates: true
    updateIntervalSeconds: 30
    options:
      path: /etc/grafana/provisioning/dashboards/climabot
  - name: Infrastructure
    orgId: 1
    folder: Infrastructure Metrics
    type: file
    disableDeletion: false
    allowUiUpdates: true
    updateIntervalSeconds: 30
    options:
      path: /etc/grafana/provisioning/dashboards/infrastructure
EOF

# ── Alerting ──────────────────────────────────────────────────
# Urgent rules go to the existing "Telegram" contact point. Informational
# rules need the Discord contact point, which is only provisioned once the
# webhook URL exists in $SECRETS/discord-webhook-url.
ALERTING="$PROV/alerting"
SECRETS=${CLIMABOT_MONITORING_SECRETS:-/root/monitoring/secrets}
install -d -m 0755 "$ALERTING"
before=$(cat "$ALERTING"/climabot-*.yaml 2>/dev/null | sha256sum)

install -m 0644 "$here/alerting/urgent.yaml" "$ALERTING/climabot-urgent.yaml"
webhook_file="$SECRETS/discord-webhook-url"
if [[ -s "$webhook_file" ]]; then
    webhook=$(tr -d '[:space:]' <"$webhook_file")
    # Readable by Grafana (uid 472, gid 0) but not by other users.
    install -m 0640 -g 0 /dev/null "$ALERTING/climabot-contactpoints.yaml"
    cat >"$ALERTING/climabot-contactpoints.yaml" <<EOF
# Managed by ClimaBot deploy/grafana/install.sh from $webhook_file.
apiVersion: 1
contactPoints:
  - orgId: 1
    name: Discord (info)
    receivers:
      - uid: climabot-discord-info
        type: discord
        disableResolveMessage: false
        settings:
          url: "$webhook"
          use_discord_username: false
          message: |
            {{ range .Alerts }}{{ if eq .Status "firing" }}🔔{{ else }}✅{{ end }} **{{ .Labels.alertname }}**: {{ .Annotations.summary }}
            {{ .Annotations.description }}
            {{ end }}
EOF
    install -m 0644 "$here/alerting/info.yaml" "$ALERTING/climabot-info.yaml"
else
    rm -f "$ALERTING/climabot-contactpoints.yaml" "$ALERTING/climabot-info.yaml"
    echo "Informational alerts disabled: put a Discord webhook URL in $webhook_file"
fi

after=$(cat "$ALERTING"/climabot-*.yaml 2>/dev/null | sha256sum)
if [[ "$before" != "$after" ]]; then
    # Grafana reads alerting provisioning only at startup.
    grafana=$(docker ps -q --filter "label=com.docker.compose.service=grafana" --filter "label=com.docker.compose.project=monitoring")
    if [[ -n "$grafana" ]]; then
        echo "Alert rules changed: restarting Grafana"
        docker restart "$grafana" >/dev/null
    fi
fi

# ── Blackbox exporter modules (uptime probes) ─────────────────
blackbox_cfg=${CLIMABOT_BLACKBOX_CONFIG:-/root/monitoring/blackbox.yml}
if [[ -f "$blackbox_cfg" ]] && ! cmp -s "$here/../monitoring/blackbox.yml" "$blackbox_cfg"; then
    # Rewrite in place: the file is bind-mounted, and replacing it would
    # leave the container reading the old copy.
    cat "$here/../monitoring/blackbox.yml" >"$blackbox_cfg"
    blackbox=$(docker ps -q --filter "label=com.docker.compose.service=blackbox" --filter "label=com.docker.compose.project=monitoring")
    [[ -n "$blackbox" ]] && docker restart "$blackbox" >/dev/null
    echo "Updated blackbox exporter modules"
fi

# An older provider scanned the whole dashboards directory, which would
# provision every dashboard twice. Retire it (it had no dashboards of its own).
old="$DASH/dashboard.yml"
if [[ -f "$old" ]] && grep -q "path: /etc/grafana/provisioning/dashboards$" "$old"; then
    mv "$old" "$PROV/dashboard.yml.retired"
    echo "Retired duplicate provider $old"
fi

echo "Installed Grafana dashboards into $DASH"
