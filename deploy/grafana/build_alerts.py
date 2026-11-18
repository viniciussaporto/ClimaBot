#!/usr/bin/env python3
"""Generates the Grafana alert rules in alerting/.

    python3 deploy/grafana/build_alerts.py

- alerting/urgent.yaml: needs action now, sent to the Telegram contact point.
- alerting/info.yaml:   worth knowing, sent to the Discord contact point.

Every rule carries four annotations, which the notification templates in
alerting/templates.yaml lay out for Telegram and Discord:

- summary:     one line, what is wrong (may use {{ $labels.x }})
- description: what it means and what it affects
- runbook:     numbered steps: what to check, how to fix, when to leave it
- value:       the reading that fired, formatted (from the query's refId A)

Provisioned by deploy/grafana/install.sh. Edit this script, not the YAML.
This repository is public: keep home-network addresses and secrets out of runbooks.
"""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent / "alerting"
PROM = "ced6krlr4hgxsb"          # Grafana's "prometheus" data source (monitoring stack)
URGENT = "Telegram"              # contact point provisioned by manassas-secrets
INFO = "Discord (info)"          # provisioned by install.sh from the webhook secret

BOT = "climabot-climabot-1"
COMPOSE = "docker compose -f /root/climabot-rust/docker-compose.yml"

# Dashboards the notifications link to (uids of the provisioned dashboards).
GRAFANA = "https://grafana.vinisaporto.de"
DASHBOARD = {
    "ClimaBot": "ded6lbvrb7lkwc",
    "vinisaporto.de": "vsite-overview",
    "Infrastructure Metrics": "docker-host",
}

# Planned downtime: the nightly `climabot update` (deploy/systemd/climabot-update.timer,
# 05:30 UTC) restarts the bot. Notifications for ClimaBot rules are held back
# in this window; anything still wrong when it ends is notified then.
NIGHTLY_MUTE = "ClimaBot nightly update"
MUTE_TIMINGS = [{
    "orgId": 1,
    "name": NIGHTLY_MUTE,
    "time_intervals": [{"times": [{"start_time": "05:25", "end_time": "06:15"}], "location": "UTC"}],
}]
RUST_BOT_RUNNING = f'count(container_last_seen{{name="{BOT}"}}) > 0'

DISK = 'container_fs_usage_bytes{id="/",device=~"/dev/(sd|vd|xvd|nvme|hd)[a-z0-9]+"}'
DISK_LIMIT = 'container_fs_limit_bytes{id="/",device=~"/dev/(sd|vd|xvd|nvme|hd)[a-z0-9]+"}'


def missing(names):
    """One series per expected container that cAdvisor hasn't seen in the last minute."""
    expected = " or ".join(f'label_replace(vector(1), "name", "{n}", "", "")' for n in names)
    return f"({expected}) unless on (name) (time() - container_last_seen < 60)"


_FORMATS = {
    "pct": "humanizePercentage .Value",
    "count": 'printf "%.0f" .Value',
    "seconds": 'printf "%.1f s" .Value',
    "duration": "humanizeDuration .Value",
    "days": 'printf "%.1f days" .Value',
}


def shown(label, fmt):
    """`value` annotation: '<label>: <refId A formatted>'. Empty when there is no value."""
    return "{{ with $values.A }}" + label + ": {{ " + _FORMATS[fmt] + " }}{{ end }}"


def steps(*lines):
    return "\n".join(f"{i}. {line}" for i, line in enumerate(lines, 1))


def rule(uid, title, expr, *, op="gt", threshold=0.0, for_="2m", summary, description, runbook,
         value=None, severity, receiver, no_data="OK", window=600):
    annotations = {"summary": summary, "description": description, "runbook": runbook}
    if value:
        annotations["value"] = value
    return {
        "uid": uid,
        "title": title,
        "condition": "C",
        "data": [
            {"refId": "A", "relativeTimeRange": {"from": window, "to": 0}, "datasourceUid": PROM,
             "model": {"refId": "A", "expr": expr, "instant": True, "range": False,
                       "intervalMs": 15000, "maxDataPoints": 43200}},
            {"refId": "C", "datasourceUid": "__expr__",
             "model": {"refId": "C", "type": "threshold", "expression": "A",
                       "conditions": [{"evaluator": {"type": op, "params": [threshold]}}]}},
        ],
        "noDataState": no_data,
        "execErrState": "Error",
        "for": for_,
        "labels": {"severity": severity},
        "annotations": annotations,
        "notification_settings": {"receiver": receiver, "group_by": ["alertname", "grafana_folder"],
                                  "repeat_interval": "4h" if severity == "critical" else "24h"},
        "isPaused": False,
    }


def urgent(*args, **kw):
    return rule(*args, severity="critical", receiver=URGENT, **kw)


def info(*args, **kw):
    kw.setdefault("for_", "10m")
    return rule(*args, severity="info", receiver=INFO, **kw)


def group(folder, name, interval, rules):
    for r in rules:
        if folder in DASHBOARD:
            # A plain annotation: Grafana's __dashboardUid__ also demands a __panelId__.
            r["annotations"]["dashboard_url"] = f"{GRAFANA}/d/{DASHBOARD[folder]}"
        if folder == "ClimaBot":
            r["notification_settings"]["mute_time_intervals"] = [NIGHTLY_MUTE]
    return {"orgId": 1, "name": name, "folder": folder, "interval": interval, "rules": rules}


# ─────────────────────────────────────────────
#  Urgent → Telegram
# ─────────────────────────────────────────────

def urgent_groups():
    bot = group("ClimaBot", "ClimaBot · urgent", "1m", [
        urgent("cb-down", "ClimaBot is down",
               'up{job="climabot"}', op="lt", threshold=1, for_="2m", no_data="Alerting",
               summary="ClimaBot is not responding",
               description="Prometheus can't scrape the bot for 2 minutes: it crashed, was stopped, or "
                           "can't start. Slash commands and hourly price checks are not running.",
               runbook=steps(
                   "`climabot status`: is climabot-climabot-1 running, restarting or exited?",
                   f"`{COMPOSE} logs --tail 100 climabot`: look for the panic or the last error.",
                   "Exited or missing: `climabot start` (pulls, rebuilds and starts the stack).",
                   "Crashes right after an update: `git -C /root/climabot-rust log --oneline -5`, "
                   "revert the bad commit on main, then `climabot update`.",
                   "Running but still down: the metrics port (9464) or Prometheus is the problem; "
                   "`docker ps --filter name=monitoring` and check the 'climabot' target in Prometheus.")),
        urgent("cb-mongo-down", "ClimaBot database is down",
               f'{missing(["climabot-mongo-1"])} and on () ({RUST_BOT_RUNNING})', for_="2m",
               summary="MongoDB of the ClimaBot stack is not running",
               description="The bot runs but can't read or save anything: tracked products, prices and "
                           "alerts are not updated.",
               runbook=steps(
                   "`docker ps -a --filter name=climabot-mongo`: exited, restarting or gone?",
                   "`docker logs climabot-mongo-1 --tail 50`: disk full and data-file corruption are "
                   "the usual causes.",
                   f"Start it: `{COMPOSE} up -d mongo`, or `climabot start` for the whole stack.",
                   "Disk full: free space first (see the 'Disk almost full' runbook), then start it.")),
    ])
    site = group("vinisaporto.de", "Website · urgent", "1m", [
        urgent("vs-website-down", "Website is down",
               'max(1 - probe_success{service=~"website|website-www"})', for_="3m",
               summary="vinisaporto.de is not reachable",
               description="The homepage fails for 3 minutes when fetched through Cloudflare (HTTP "
                           "error, TLS problem or timeout).",
               runbook=steps(
                   "`curl -sI https://vinisaporto.de`: a 52x status comes from Cloudflare "
                   "(520-523: Caddy unreachable or refusing; 525/526: TLS between Cloudflare and Caddy).",
                   "`systemctl status caddy` and `journalctl -u caddy --since -15min`.",
                   "Bad config after an edit: `caddy validate --config /etc/caddy/Caddyfile`, fix, "
                   "then `systemctl reload caddy`.",
                   "Caddy fine but the site 404s or 500s: `ls /srv/vinisaporto/public`, and redeploy with "
                   "`vs-update`.",
                   "Everything fine on the server: check the Cloudflare dashboard (DNS, SSL mode, outages).")),
        urgent("vs-api-down", "Website API is down",
               'max(1 - probe_success{service="api"})', for_="3m",
               summary="vinisaporto.de/api is failing",
               description="/api/challenge fails for 3 minutes: the contact form and the CV download "
                           "don't work.",
               runbook=steps(
                   "`docker ps -a --filter name=vsite-api` and `docker logs vsite-api --tail 50`.",
                   "Restart it: `docker compose -f /opt/vinisaporto/deploy/compose.yaml up -d api`.",
                   "Container healthy but still failing: Caddy proxies /api/* to 127.0.0.1:18081, so "
                   "`curl -s 127.0.0.1:18081/api/challenge` tells Caddy and the API apart.")),
        urgent("vs-status-page-down", "Status page is down",
               'max(1 - probe_success{service="status-page"})', for_="3m",
               summary="vinisaporto.de/status is not reachable",
               description="The public status page fails for 3 minutes. If 'Website is down' fires too, "
                           "start there.",
               runbook=steps(
                   "`curl -sI https://vinisaporto.de/status`.",
                   "Only this page fails: the static build is missing it; redeploy with `vs-update`.")),
        urgent("vs-status-degraded", "Status page reports a problem",
               'max(1 - probe_success{service="status-api"})', for_="5m",
               summary="/api/status is failing or not reporting ok",
               description="/api/status fails, or reports one of the website's services as down, for "
                           "5 minutes.",
               runbook=steps(
                   "Open https://vinisaporto.de/status to see which service is down.",
                   "`curl -s https://vinisaporto.de/api/status` shows the raw checks.",
                   "Restart the failing container from /opt/vinisaporto/deploy/compose.yaml "
                   "(`docker compose ... up -d <service>`) and read its logs.")),
        urgent("vs-cert-expiring", "TLS certificate expires within 7 days",
               "min((probe_ssl_earliest_cert_expiry - time()) / 86400)", op="lt", threshold=7, for_="1h",
               summary="A certificate expires in less than 7 days",
               description="The probes go through Cloudflare, so this is the certificate visitors see: "
                           "Cloudflare's edge certificate, which renews itself.",
               value=shown("Expires in", "days"),
               runbook=steps(
                   "`echo | openssl s_client -connect vinisaporto.de:443 -servername vinisaporto.de "
                   "2>/dev/null | openssl x509 -noout -issuer -enddate` shows who issued it.",
                   "Cloudflare-issued: Cloudflare dashboard, SSL/TLS, Edge Certificates; check the "
                   "Universal SSL status and re-order it if it's stuck.",
                   "Not Cloudflare (proxy turned off?): it's Caddy's, so `journalctl -u caddy | grep -i "
                   "acme` for the renewal error.")),
    ])
    host = group("Infrastructure Metrics", "Server · urgent", "1m", [
        urgent("srv-disk-full", "Disk almost full",
               f"max({DISK} / {DISK_LIMIT})", threshold=0.9, for_="10m",
               summary="A disk is over 90% full",
               description="Databases, Prometheus and logs fail to write once it's full, and services "
                           "start crashing.",
               value=shown("Used", "pct"),
               runbook=steps(
                   "`df -h /` and `docker system df` to see what's big.",
                   "Quick wins: `docker builder prune -af` and `docker image prune -af`.",
                   "`journalctl --disk-usage` (capped at 2G) and `du -xh /var/lib/docker --max-depth 1 | "
                   "sort -h | tail`.",
                   "Prometheus keeps 180 days (up to 40 GB): lower the retention in "
                   "/root/monitoring/docker-compose.yml if it's the culprit.")),
        urgent("srv-memory-critical", "Server out of memory",
               'max(container_memory_working_set_bytes{id="/"}) / max(machine_memory_bytes)',
               threshold=0.95, for_="5m",
               summary="Server memory is over 95%",
               description="Only the 4 GB swap file is left before the kernel starts killing processes.",
               value=shown("Used", "pct"),
               runbook=steps(
                   "`docker stats --no-stream` for the biggest container.",
                   "The scraper browsers (FlareSolverr, Byparr) leak memory: "
                   f"`{COMPOSE} restart flaresolverr byparr`.",
                   "Restart whatever else grew; then look at 'Docker Host & Containers' for the trend.")),
    ])
    return [bot, site, host]


# ─────────────────────────────────────────────
#  Informational → Discord
# ─────────────────────────────────────────────

def info_groups():
    bot = group("ClimaBot", "ClimaBot · info", "5m", [
        info("cb-restarted", "ClimaBot restarted",
             'changes(process_start_time_seconds{job="climabot"}[15m])', for_="0s",
             summary="The bot process restarted",
             description="Outside the nightly update window, so it was a crash or a manual restart.",
             runbook=steps(
                 f"`{COMPOSE} logs --tail 200 climabot | grep -iE 'panic|error'` for the cause.",
                 "One restart after a crash is fine; repeated ones raise 'Container crash-looping'.")),
        info("cb-command-errors", "ClimaBot command errors",
             'sum(increase(discord_command_total{status="error"}[1h])) / '
             'sum(increase(discord_command_total{status="received"}[1h])) '
             'and on () sum(increase(discord_command_total{status="received"}[1h])) >= 5',
             threshold=0.2,
             summary="More than 20% of commands failed in the last hour",
             description="Users get error replies to slash commands.",
             value=shown("Failed", "pct"),
             runbook=steps(
                 "'Command errors' panel on the ClimaBot dashboard: which command?",
                 "`climabot logs` and try the command in Discord to reproduce.",
                 "Weather commands: see 'Weather API errors'. /pt commands: see 'Price checks failing'.")),
        info("cb-weather-api-errors", "Weather API errors",
             'sum(increase(weather_api_requests_total{status="error"}[30m])) / '
             'sum(increase(weather_api_requests_total[30m])) '
             'and on () sum(increase(weather_api_requests_total[30m])) >= 3',
             threshold=0.2,
             summary="More than 20% of weather/geocoding API calls failed",
             description="Weather commands fail or answer without a location.",
             value=shown("Failed", "pct"),
             runbook=steps(
                 "`climabot logs | grep -iE 'opencage|open-meteo'` for the status codes.",
                 "OpenCage 402/429: the daily quota is used up; it resets at midnight UTC.",
                 "Open-Meteo 5xx: their outage, nothing to do but wait.")),
        info("cb-price-overdue", "Price checks falling behind",
             "price_checks_overdue", for_="15m",
             summary="Some hourly price checks are more than 5 minutes late",
             description="The scheduler can't keep up, or the browsers are slow or stuck.",
             value=shown("Overdue checks", "count"),
             runbook=steps(
                 "Browser health: `docker stats --no-stream climabot-flaresolverr-1 climabot-byparr-1`.",
                 f"Stuck or near their memory limit: `{COMPOSE} restart flaresolverr byparr`.",
                 "Still behind: too many products for one hour of checks; look at check durations on "
                 "the ClimaBot dashboard.")),
        info("cb-price-failures", "Price checks failing",
             'sum(increase(price_checks_total{status="error"}[6h])) / sum(increase(price_checks_total[6h])) '
             'and on () sum(increase(price_checks_total[6h])) >= 6',
             threshold=0.5, for_="30m",
             summary="More than half of the price checks failed in the last 6 hours",
             description="Price drops are not being noticed for the failing shops.",
             value=shown("Failed", "pct"),
             runbook=steps(
                 "'Fetch success rate by method' on the ClimaBot dashboard: one shop or all of them?",
                 "One shop: it changed its page or its bot protection; `climabot failed-sites` and the "
                 "logs show the reason.",
                 "Cloudflare-protected shops block this server's datacenter IP; they need a residential "
                 "proxy in SCRAPER_PROXY.",
                 "All shops: the browsers are down, see 'Scraper browser down'.")),
        info("cb-browser-memory", "Scraper browser near memory limit",
             'max by (container_label_com_docker_compose_service) '
             '(container_memory_working_set_bytes{container_label_com_docker_compose_project="climabot"} / '
             '(container_spec_memory_limit_bytes{container_label_com_docker_compose_project="climabot"} > 0))',
             threshold=0.9,
             summary="{{ $labels.container_label_com_docker_compose_service }} is above 90% of its memory "
                     "limit",
             description="It will be OOM-killed and restarted when it reaches the limit, failing the "
                         "checks in flight.",
             value=shown("Of its limit", "pct"),
             runbook=steps(
                 f"Restart it before it's killed: `{COMPOSE} restart "
                 "{{ $labels.container_label_com_docker_compose_service }}`.",
                 "If it happens daily, raise FLARESOLVERR_MEM_LIMIT or BYPARR_MEM_LIMIT in "
                 "/root/climabot-rust/.env and `climabot update`.")),
        info("cb-browsers-down", "Scraper browser down",
             f'{missing(["climabot-flaresolverr-1", "climabot-byparr-1"])} and on () ({RUST_BOT_RUNNING})',
             for_="5m",
             summary="{{ $labels.name }} is not running",
             description="Shops behind bot protection can't be checked until it's back.",
             runbook=steps(
                 "`docker ps -a --filter name={{ $labels.name }}` and `docker logs {{ $labels.name }} "
                 "--tail 50`.",
                 f"Start it: `{COMPOSE} up -d` (starts whatever is missing).")),
    ])
    site = group("vinisaporto.de", "Website · info", "5m", [
        info("vs-slow", "Website slow",
             'max(probe_duration_seconds{service=~"website|status-page|api"})', threshold=3,
             summary="The website takes more than 3 seconds to answer",
             description="Measured from the server through Cloudflare, TLS included.",
             value=shown("Slowest probe", "seconds"),
             runbook=steps(
                 "Server busy? Check 'Server CPU high' / memory on 'Docker Host & Containers'.",
                 "Only the API slow: `docker logs vsite-api --tail 50`.",
                 "Server idle: likely Cloudflare or the network; check status.cloudflare.com.")),
        info("vs-cert-renewal", "TLS certificate renewal overdue",
             "min((probe_ssl_earliest_cert_expiry - time()) / 86400)", op="lt", threshold=21, for_="6h",
             summary="A certificate expires in less than 21 days",
             description="Edge certificates normally renew around 30 days before expiry. Early warning "
                         "for 'TLS certificate expires within 7 days'.",
             value=shown("Expires in", "days"),
             runbook=steps(
                 "Same checks as 'TLS certificate expires within 7 days', without the hurry.")),
        info("vs-grafana-down", "Grafana public URL down",
             'max(1 - probe_success{service="grafana"})', for_="10m",
             summary="grafana.vinisaporto.de is not reachable from outside",
             description="Grafana itself still runs (it sent this), so the route to it is broken.",
             runbook=steps(
                 "`curl -sI https://grafana.vinisaporto.de/api/health` for the status code.",
                 "`cat /etc/caddy/sites/grafana.caddy` and `journalctl -u caddy --since -30min`.",
                 "`systemctl reload caddy` after fixing; check the DNS record in Cloudflare.")),
        info("vs-telemetry-down", "Website telemetry down",
             missing(["vsite-prometheus", "vsite-loki", "vsite-alloy", "vsite-crowdsec"]), for_="5m",
             summary="{{ $labels.name }} is not running",
             description="The status page and website statistics lose data (CrowdSec: no bot "
                         "blocking) until it's back.",
             runbook=steps(
                 "`docker logs {{ $labels.name }} --tail 50`.",
                 "`docker compose -f /opt/vinisaporto/deploy/compose.yaml up -d` starts what's missing.")),
    ])
    host = group("Infrastructure Metrics", "Server · info", "5m", [
        info("srv-cpu-high", "Server CPU high",
             'sum(rate(container_cpu_usage_seconds_total{id="/"}[5m])) / max(machine_cpu_cores)',
             threshold=0.9, for_="15m",
             summary="Server CPU above 90% for 15 minutes",
             description="Everything on the server slows down: price checks, the website, Grafana.",
             value=shown("CPU", "pct"),
             runbook=steps(
                 "'Docker Host & Containers' or `docker stats --no-stream` for the busiest container.",
                 "Usually a scraper browser spinning on a page: restart it.",
                 "Nothing in Docker: `top` for host processes (a stuck backup or update).")),
        info("srv-memory-high", "Server memory high",
             'max(container_memory_working_set_bytes{id="/"}) / max(machine_memory_bytes)',
             threshold=0.85, for_="15m",
             summary="Server memory above 85% for 15 minutes",
             description="Early warning for 'Server out of memory'.",
             value=shown("Used", "pct"),
             runbook=steps("Same steps as 'Server out of memory', without the hurry.")),
        info("srv-disk-high", "Disk filling up",
             f"max({DISK} / {DISK_LIMIT})", threshold=0.8, for_="1h",
             summary="A disk is over 80% full",
             description="Early warning for 'Disk almost full'.",
             value=shown("Used", "pct"),
             runbook=steps("`docker builder prune -af` and `docker image prune -af` usually free "
                           "enough; otherwise follow 'Disk almost full'.")),
        # Single restarts are normal (deploys, `vs-update` reloading CrowdSec/GeoIP, config
        # changes) and cAdvisor can't tell them from crashes, so only loops and OOM kills alert.
        info("srv-container-restarting", "Container crash-looping",
             'max by (name) (changes(container_start_time_seconds{name!=""}[15m]))', threshold=2,
             for_="0s",
             summary="{{ $labels.name }} restarted 3+ times in 15 minutes",
             description="It keeps exiting and being restarted.",
             value=shown("Restarts in 15 min", "count"),
             runbook=steps(
                 "`docker logs {{ $labels.name }} --tail 100` for why it exits.",
                 "`docker inspect {{ $labels.name }} | grep -E 'ExitCode|OOMKilled'`: exit code 137 or "
                 "OOMKilled true means memory.",
                 "Started after an update: roll back that service's last change.")),
        info("srv-container-oom", "Container ran out of memory",
             'sum by (name) (increase(container_oom_events_total{name!=""}[15m]))', for_="0s",
             summary="{{ $labels.name }} was killed for running out of memory",
             description="The kernel OOM-killed a process in it (it may have restarted fine).",
             runbook=steps(
                 "`docker stats --no-stream {{ $labels.name }}` against its limit.",
                 "Once: ignore. Repeats: raise its memory limit or look for a leak on "
                 "'Docker Host & Containers'.")),
        info("srv-monitoring-down", "Monitoring component down",
             'up{job=~"cadvisor|blackbox"} < 1 or ' + missing(["monitoring-cadvisor-1", "monitoring-blackbox-1"]),
             op="gt", threshold=-1, for_="10m",
             summary="{{ $labels.job }}{{ $labels.name }} is not working",
             description="Container and uptime alerts can't see their data until it's back, so they "
                         "stay silent.",
             runbook=steps(
                 "`docker ps -a --filter name=monitoring` and the logs of the missing one.",
                 "`cd /root/monitoring && docker compose up -d` starts it again.")),
    ])
    apps = group("Infrastructure Metrics", "Self-hosted apps · info", "5m", [
        info("app-unreachable", "Self-hosted app unreachable",
             'max by (service, instance) (1 - probe_success{service=~"home-assistant|immich|amp"})',
             for_="10m",
             summary="{{ $labels.service }} is not reachable from outside",
             description="{{ $labels.instance }} fails through Cloudflare: its phone apps and remote "
                         "access don't work. At home on the LAN it may still be fine.",
             runbook=steps(
                 "`curl -sI {{ $labels.instance }}`: a 52x status comes from Cloudflare (Caddy on this "
                 "server is unreachable or can't reach the app).",
                 "`journalctl -u caddy --since -30min | grep -i error` and `/var/log/caddy/` for the "
                 "site's access log.",
                 "Home Assistant and Immich are reached over Tailscale: `tailscale status` must show "
                 "their nodes, and the app must answer on the tailnet address used in "
                 "/etc/caddy/sites/.",
                 "Node offline in Tailscale: the VM or the home connection is down; check it at home.")),
    ])
    return [bot, site, host, apps]


# Rules that were removed from this script. File provisioning never deletes a rule
# that just disappears from the file, so they are deleted explicitly.
RETIRED = [
    "cb-gateway-latency",   # "Discord gateway slow": not actionable, the bot reconnects by itself
]


def write(name, groups, mute_timings=None, delete=()):
    OUT.mkdir(exist_ok=True)
    doc = {"apiVersion": 1, "groups": groups}
    if mute_timings:
        doc["muteTimes"] = mute_timings
    if delete:
        doc["deleteRules"] = [{"orgId": 1, "uid": uid} for uid in delete]
    header = ("# Generated by deploy/grafana/build_alerts.py: edit that, not this file.\n")
    # JSON is valid YAML and keeps PromQL quoting unambiguous.
    (OUT / name).write_text(header + json.dumps(doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print("wrote", OUT / name, sum(len(g["rules"]) for g in groups), "rules")


if __name__ == "__main__":
    # Mute timings go with the urgent rules: that file is always installed.
    write("urgent.yaml", urgent_groups(), MUTE_TIMINGS)
    write("info.yaml", info_groups(), delete=RETIRED)
