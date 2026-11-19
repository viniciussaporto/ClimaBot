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
alerting_checksum() {
    local f
    for f in "$ALERTING"/climabot-*.yaml; do
        if [[ -f "$f" ]]; then cat "$f"; fi
    done | sha256sum
}
before=$(alerting_checksum)
# Keep the current files: a provisioning error (e.g. a bad template) stops
# Grafana from starting, so they are put back if it doesn't come up healthy.
previous=$(mktemp -d)
trap 'rm -rf "$previous"' EXIT
cp -p "$ALERTING"/climabot-*.yaml "$previous"/ 2>/dev/null || true

install -m 0644 "$here/alerting/urgent.yaml" "$ALERTING/climabot-urgent.yaml"
# Notification templates (manassas.*) used by both contact points: the Telegram
# one comes from manassas-secrets and references manassas.telegram.message.
install -m 0644 "$here/alerting/templates.yaml" "$ALERTING/climabot-templates.yaml"
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
          # Templates from alerting/templates.yaml (installed above).
          title: '{{ template "manassas.discord.title" . }}'
          message: '{{ template "manassas.discord.message" . }}'
EOF
    install -m 0644 "$here/alerting/info.yaml" "$ALERTING/climabot-info.yaml"
else
    rm -f "$ALERTING/climabot-contactpoints.yaml" "$ALERTING/climabot-info.yaml"
    echo "Informational alerts disabled: put a Discord webhook URL in $webhook_file"
fi

after=$(alerting_checksum)
if [[ "$before" != "$after" ]]; then
    # Grafana reads alerting provisioning only at startup.
    grafana=$(docker ps -q --filter "label=com.docker.compose.service=grafana" --filter "label=com.docker.compose.project=monitoring")
    if [[ -n "$grafana" ]]; then
        echo "Alert rules changed: restarting Grafana"
        docker restart "$grafana" >/dev/null
        healthy() {
            for _ in $(seq 1 40); do
                curl -sf http://127.0.0.1:3000/api/health >/dev/null && return 0
                sleep 3
            done
            return 1
        }
        if ! healthy; then
            echo "Grafana did not come back healthy: restoring the previous alerting files" >&2
            docker logs --tail 50 "$grafana" 2>&1 | grep -iE "level=error" | tail -3 >&2 || true
            rm -f "$ALERTING"/climabot-*.yaml
            cp -p "$previous"/climabot-*.yaml "$ALERTING"/ 2>/dev/null || true
            docker restart "$grafana" >/dev/null
            healthy && echo "Grafana is healthy again with the previous alerting files" >&2 \
                || echo "Grafana is still unhealthy: check 'docker logs $grafana'" >&2
            exit 1
        fi
        echo "Grafana is healthy"
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
