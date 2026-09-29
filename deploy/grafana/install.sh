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

# An older provider scanned the whole dashboards directory, which would
# provision every dashboard twice. Retire it (it had no dashboards of its own).
old="$DASH/dashboard.yml"
if [[ -f "$old" ]] && grep -q "path: /etc/grafana/provisioning/dashboards$" "$old"; then
    mv "$old" "$PROV/dashboard.yml.retired"
    echo "Retired duplicate provider $old"
fi

echo "Installed Grafana dashboards into $DASH"
