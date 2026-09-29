#!/usr/bin/env python3
"""Generates the Grafana alert rules in alerting/.

    python3 deploy/grafana/build_alerts.py

- alerting/urgent.yaml: needs action now, sent to the Telegram contact point.
- alerting/info.yaml:   worth knowing, sent to the Discord contact point.

Provisioned by deploy/grafana/install.sh. Edit this script, not the YAML.
"""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent / "alerting"
PROM = "ced6krlr4hgxsb"          # Grafana's "prometheus" data source (monitoring stack)
URGENT = "Telegram"              # existing contact point
INFO = "Discord (info)"          # provisioned by install.sh from the webhook secret

BOT = "climabot-climabot-1"
RUST_BOT_RUNNING = f'count(container_last_seen{{name="{BOT}"}}) > 0'


def missing(names):
    """One series per expected container that cAdvisor hasn't seen in the last minute."""
    expected = " or ".join(f'label_replace(vector(1), "name", "{n}", "", "")' for n in names)
    return f"({expected}) unless on (name) (time() - container_last_seen < 60)"


def rule(uid, title, expr, *, op="gt", threshold=0.0, for_="2m", summary, description, severity,
         receiver, no_data="OK", window=600):
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
        "annotations": {"summary": summary, "description": description},
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
                           "can't start. Check `climabot status` and `docker compose -f "
                           "/root/climabot-rust/docker-compose.yml logs climabot`."),
        urgent("cb-mongo-down", "ClimaBot database is down",
               f'{missing(["climabot-mongo-1"])} and on () ({RUST_BOT_RUNNING})', for_="2m",
               summary="MongoDB of the ClimaBot stack is not running",
               description="Price tracking can't read or save anything. "
                           "Start it with `climabot update` or check `docker ps -a`."),
    ])
    site = group("vinisaporto.de", "Website · urgent", "1m", [
        urgent("vs-website-down", "Website is down",
               'max(1 - probe_success{service=~"website|website-www"})', for_="3m",
               summary="vinisaporto.de is not reachable",
               description="The homepage fails for 3 minutes (HTTP error, TLS problem or timeout). "
                           "Check Caddy: `systemctl status caddy`."),
        urgent("vs-api-down", "Website API is down",
               'max(1 - probe_success{service="api"})', for_="3m",
               summary="vinisaporto.de/api is failing",
               description="/api/challenge fails for 3 minutes, so the contact form can't be used. "
                           "Check `docker logs vsite-api`."),
        urgent("vs-status-page-down", "Status page is down",
               'max(1 - probe_success{service="status-page"})', for_="3m",
               summary="vinisaporto.de/status is not reachable",
               description="The public status page fails for 3 minutes."),
        urgent("vs-status-degraded", "Status page reports a problem",
               'max(1 - probe_success{service="status-api"})', for_="5m",
               summary="/api/status is failing or not reporting ok",
               description="/api/status fails, or reports one of the website's services as down, for "
                           "5 minutes. Open https://vinisaporto.de/status to see which one."),
        urgent("vs-cert-expiring", "TLS certificate expires within 7 days",
               "min((probe_ssl_earliest_cert_expiry - time()) / 86400)", op="lt", threshold=7, for_="1h",
               summary="A certificate expires in less than 7 days",
               description="Caddy renews certificates automatically about 30 days before expiry, so "
                           "renewal is failing. Check `journalctl -u caddy`."),
    ])
    host = group("Infrastructure Metrics", "Server · urgent", "1m", [
        urgent("srv-disk-full", "Disk almost full",
               'max(container_fs_usage_bytes{id="/",device=~"/dev/(sd|vd|xvd|nvme|hd)[a-z0-9]+"} / '
               'container_fs_limit_bytes{id="/",device=~"/dev/(sd|vd|xvd|nvme|hd)[a-z0-9]+"})',
               threshold=0.9, for_="10m",
               summary="A disk is over 90% full",
               description="Free space before databases and logs fail to write. `docker system df` and "
                           "`docker builder prune` usually help."),
        urgent("srv-memory-critical", "Server out of memory",
               'max(container_memory_working_set_bytes{id="/"}) / max(machine_memory_bytes)',
               threshold=0.95, for_="5m",
               summary="Server memory is over 95%",
               description="There is no swap: the kernel will start killing processes."),
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
             description="Expected after `climabot update`; otherwise look for a crash in the logs."),
        info("cb-command-errors", "ClimaBot command errors",
             'sum(increase(discord_command_total{status="error"}[1h])) / '
             'sum(increase(discord_command_total{status="received"}[1h])) '
             'and on () sum(increase(discord_command_total{status="received"}[1h])) >= 5',
             threshold=0.2,
             summary="More than 20% of commands failed in the last hour",
             description="See the 'Command errors' panel on the ClimaBot dashboard."),
        info("cb-weather-api-errors", "Weather API errors",
             'sum(increase(weather_api_requests_total{status="error"}[30m])) / '
             'sum(increase(weather_api_requests_total[30m])) '
             'and on () sum(increase(weather_api_requests_total[30m])) >= 3',
             threshold=0.2,
             summary="More than 20% of weather/geocoding API calls failed",
             description="OpenCage (quota?) or Open-Meteo is having problems."),
        info("cb-price-overdue", "Price checks falling behind",
             "price_checks_overdue", for_="15m",
             summary="Some hourly price checks are more than 5 minutes late",
             description="The scheduler can't keep up, or the browsers are slow or stuck."),
        info("cb-price-failures", "Price checks failing",
             'sum(increase(price_checks_total{status="error"}[6h])) / sum(increase(price_checks_total[6h])) '
             'and on () sum(increase(price_checks_total[6h])) >= 6',
             threshold=0.5, for_="30m",
             summary="More than half of the price checks failed in the last 6 hours",
             description="A shop changed its page or its bot protection. See 'Fetch success rate by "
                         "method' on the ClimaBot dashboard."),
        info("cb-browser-memory", "Scraper browser near memory limit",
             'max by (container_label_com_docker_compose_service) '
             '(container_memory_working_set_bytes{container_label_com_docker_compose_project="climabot"} / '
             '(container_spec_memory_limit_bytes{container_label_com_docker_compose_project="climabot"} > 0))',
             threshold=0.9,
             summary="{{ $labels.container_label_com_docker_compose_service }} is above 90% of its memory "
                     "limit",
             description="It will be OOM-killed and restarted if it reaches the limit."),
        info("cb-browsers-down", "Scraper browser down",
             f'{missing(["climabot-flaresolverr-1", "climabot-byparr-1"])} and on () ({RUST_BOT_RUNNING})',
             for_="5m",
             summary="{{ $labels.name }} is not running",
             description="Shops behind bot protection can't be checked until it's back."),
        info("cb-gateway-latency", "Discord gateway slow",
             "discord_gateway_latency_seconds", threshold=1, for_="15m",
             summary="Discord gateway latency is above 1 second",
             description="Usually a Discord or network problem; the bot reconnects by itself."),
    ])
    site = group("vinisaporto.de", "Website · info", "5m", [
        info("vs-slow", "Website slow",
             'max(probe_duration_seconds{service=~"website|status-page|api"})', threshold=3,
             summary="The website takes more than 3 seconds to answer",
             description="Measured from the server itself, TLS included."),
        info("vs-cert-renewal", "TLS certificate renewal overdue",
             "min((probe_ssl_earliest_cert_expiry - time()) / 86400)", op="lt", threshold=21, for_="6h",
             summary="A certificate expires in less than 21 days",
             description="Caddy should have renewed it by now. Check `journalctl -u caddy`."),
        info("vs-grafana-down", "Grafana public URL down",
             'max(1 - probe_success{service="grafana"})', for_="10m",
             summary="grafana.vinisaporto.de is not reachable from outside",
             description="Grafana itself still runs (it sent this); check the Caddy route."),
        info("vs-telemetry-down", "Website telemetry down",
             missing(["vsite-prometheus", "vsite-loki", "vsite-alloy", "vsite-crowdsec"]), for_="5m",
             summary="{{ $labels.name }} is not running",
             description="The status page and website statistics lose data (CrowdSec: no bot "
                         "blocking) until it's back."),
    ])
    host = group("Infrastructure Metrics", "Server · info", "5m", [
        info("srv-cpu-high", "Server CPU high",
             'sum(rate(container_cpu_usage_seconds_total{id="/"}[5m])) / max(machine_cpu_cores)',
             threshold=0.9, for_="15m",
             summary="Server CPU above 90% for 15 minutes",
             description="See 'Docker Host & Containers' for the busiest container."),
        info("srv-memory-high", "Server memory high",
             'max(container_memory_working_set_bytes{id="/"}) / max(machine_memory_bytes)',
             threshold=0.85, for_="15m",
             summary="Server memory above 85% for 15 minutes",
             description="See 'Docker Host & Containers' for the largest container."),
        info("srv-disk-high", "Disk filling up",
             'max(container_fs_usage_bytes{id="/",device=~"/dev/(sd|vd|xvd|nvme|hd)[a-z0-9]+"} / '
             'container_fs_limit_bytes{id="/",device=~"/dev/(sd|vd|xvd|nvme|hd)[a-z0-9]+"})',
             threshold=0.8, for_="1h",
             summary="A disk is over 80% full",
             description="`docker builder prune` and `docker image prune` free space."),
        info("srv-container-restarting", "Container restarting",
             'max by (name) (changes(container_start_time_seconds{name!=""}[15m]))',
             for_="0s",
             summary="{{ $labels.name }} restarted",
             description="It exited and was restarted: a crash, running out of memory, or a manual "
                         "`docker restart` (several in a row = crash loop). Check `docker logs "
                         "{{ $labels.name }}`. Recreating a container (e.g. `climabot update`) doesn't "
                         "trigger this."),
        info("srv-monitoring-down", "Monitoring component down",
             'up{job=~"cadvisor|blackbox"} < 1 or ' + missing(["monitoring-cadvisor-1", "monitoring-blackbox-1"]),
             op="gt", threshold=-1, for_="10m",
             summary="{{ $labels.job }}{{ $labels.name }} is not working",
             description="Some alerts can't see their data until it's back."),
    ])
    return [bot, site, host]


def write(name, groups):
    OUT.mkdir(exist_ok=True)
    doc = {"apiVersion": 1, "groups": groups}
    header = ("# Generated by deploy/grafana/build_alerts.py: edit that, not this file.\n")
    # JSON is valid YAML and keeps PromQL quoting unambiguous.
    (OUT / name).write_text(header + json.dumps(doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print("wrote", OUT / name, sum(len(g["rules"]) for g in groups), "rules")


if __name__ == "__main__":
    write("urgent.yaml", urgent_groups())
    write("info.yaml", info_groups())
