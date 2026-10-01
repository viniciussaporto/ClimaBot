#!/usr/bin/env python3
"""Generates the Grafana dashboards in this directory.

    python3 deploy/grafana/build_dashboards.py

Writes climabot.json and docker-host.json, provisioned into Grafana by
deploy/grafana/install.sh. Edit this script rather than the JSON.
"""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent
DS = {"type": "prometheus", "uid": "${datasource}"}
# The website stack's Loki (fixed uid from its install-grafana.sh); its Alloy
# ships the bot's JSON log files there as {job="climabot", level=...}.
LOKI = {"type": "loki", "uid": "vsite-loki"}
LOGS = '{job="climabot"}'
BOT = 'job="climabot"'
STACK = 'container_label_com_docker_compose_project="climabot"'
SVC = "container_label_com_docker_compose_service"
PROJ = "container_label_com_docker_compose_project"
# Real block devices only; cAdvisor also reports tmpfs, overlay and /run.
DISKS = 'id="/",device=~"/dev/(sd|vd|xvd|nvme|hd)[a-z0-9]+"'


class Layout:
    """Places panels left to right, wrapping at 24 columns."""

    def __init__(self):
        self.panels = []
        self.x = 0
        self.y = 0
        self.row_h = 0
        self.next_id = 1

    def add(self, panel, w, h):
        if self.x + w > 24:
            self.newline()
        panel["id"] = self.next_id
        self.next_id += 1
        panel["gridPos"] = {"x": self.x, "y": self.y, "w": w, "h": h}
        self.panels.append(panel)
        self.x += w
        self.row_h = max(self.row_h, h)

    def newline(self):
        self.y += self.row_h
        self.x = 0
        self.row_h = 0

    def row(self, title, collapsed=False):
        self.newline()
        self.add({"type": "row", "title": title, "collapsed": collapsed, "panels": []}, 24, 1)
        self.newline()


def target(expr, legend="", ref="A", instant=False, fmt="time_series"):
    t = {"datasource": DS, "expr": expr, "legendFormat": legend or "__auto", "refId": ref, "format": fmt}
    if instant:
        t.update({"instant": True, "range": False})
    return t


def thresholds(*steps):
    """steps: ("green", None), ("orange", 1), ..."""
    return {"mode": "absolute", "steps": [{"color": c, "value": v} for c, v in steps]}


def stat(title, expr, unit="short", desc="", steps=(("green", None),), decimals=None, mappings=None,
         color_mode="value", graph=True):
    defaults = {"unit": unit, "thresholds": thresholds(*steps), "color": {"mode": "thresholds"}}
    if decimals is not None:
        defaults["decimals"] = decimals
    if mappings:
        defaults["mappings"] = mappings
    defaults["noValue"] = "–"
    return {
        "type": "stat", "title": title, "description": desc, "datasource": DS,
        "targets": [target(expr, instant=not graph)],
        "fieldConfig": {"defaults": defaults, "overrides": []},
        "options": {
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "colorMode": color_mode, "graphMode": "area" if graph else "none",
            "justifyMode": "auto", "textMode": "value", "orientation": "auto",
        },
    }


def gauge(title, expr, desc="", unit="percentunit", steps=(("green", None), ("orange", 0.75), ("red", 0.9))):
    return {
        "type": "gauge", "title": title, "description": desc, "datasource": DS,
        "targets": [target(expr, instant=True)],
        "fieldConfig": {"defaults": {"unit": unit, "min": 0, "max": 1 if unit == "percentunit" else None,
                                     "thresholds": thresholds(*steps), "color": {"mode": "thresholds"},
                                     "noValue": "–"}, "overrides": []},
        "options": {"reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
                    "showThresholdMarkers": True, "showThresholdLabels": False},
    }


def timeseries(title, targets, unit="short", desc="", stack=False, bars=False, min0=True, overrides=None,
               legend_calcs=("mean", "max"), fill=10):
    custom = {
        "drawStyle": "bars" if bars else "line",
        "lineWidth": 1 if bars else 2,
        "fillOpacity": 80 if bars else fill,
        "gradientMode": "none",
        "showPoints": "never",
        "spanNulls": True,
        "stacking": {"mode": "normal" if stack else "none", "group": "A"},
        "axisSoftMin": 0 if min0 else None,
    }
    return {
        "type": "timeseries", "title": title, "description": desc, "datasource": DS,
        "targets": targets,
        "fieldConfig": {"defaults": {"unit": unit, "custom": custom, "color": {"mode": "palette-classic"},
                                     "noValue": "No data in this time range"},
                        "overrides": overrides or []},
        "options": {
            "legend": {"displayMode": "table", "placement": "bottom", "showLegend": True,
                       "calcs": list(legend_calcs)},
            "tooltip": {"mode": "multi", "sort": "desc"},
        },
    }


def bargauge(title, expr, legend, unit="short", desc="", steps=(("blue", None),), max_=None, sort=True):
    fc = {"unit": unit, "min": 0, "thresholds": thresholds(*steps), "color": {"mode": "thresholds"},
          "noValue": "None in this time range"}
    if max_ is not None:
        fc["max"] = max_
    q = f"sort_desc({expr})" if sort else expr
    return {
        "type": "bargauge", "title": title, "description": desc, "datasource": DS,
        "targets": [target(q, legend, instant=True)],
        "fieldConfig": {"defaults": fc, "overrides": []},
        "options": {"reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
                    "orientation": "horizontal", "displayMode": "gradient", "showUnfilled": True,
                    "valueMode": "color", "namePlacement": "auto", "minVizHeight": 16, "maxVizHeight": 40},
    }


def table(title, queries, rename, desc="", units=None, sort_by=None):
    """queries: [(refId, expr)], all instant, merged into one row per label set."""
    overrides = []
    for col, unit in (units or {}).items():
        overrides.append({"matcher": {"id": "byName", "options": col},
                          "properties": [{"id": "unit", "value": unit}]})
    return {
        "type": "table", "title": title, "description": desc, "datasource": DS,
        "targets": [target(expr, ref=ref, instant=True, fmt="table") for ref, expr in queries],
        "transformations": [
            {"id": "merge", "options": {}},
            {"id": "organize", "options": {"excludeByName": {"Time": True}, "renameByName": rename}},
        ],
        "fieldConfig": {"defaults": {"custom": {"align": "auto", "cellOptions": {"type": "auto"}},
                                     "noValue": "–"}, "overrides": overrides},
        "options": {"showHeader": True, "cellHeight": "sm",
                    "sortBy": [{"displayName": sort_by, "desc": True}] if sort_by else []},
    }


def loki_target(expr, legend="", ref="A"):
    return {"datasource": LOKI, "expr": expr, "legendFormat": legend, "refId": ref, "queryType": "range"}


def logs(title, expr, desc=""):
    return {
        "type": "logs", "title": title, "description": desc, "datasource": LOKI,
        "targets": [loki_target(expr)],
        "options": {"showTime": True, "wrapLogMessage": True, "enableLogDetails": True,
                    "sortOrder": "Descending", "dedupStrategy": "none", "showLabels": False,
                    "showCommonLabels": False, "prettifyLogMessage": False},
    }


def text(title, content):
    return {"type": "text", "title": title, "options": {"mode": "markdown", "content": content}}


def datasource_var():
    return {"name": "datasource", "label": "Data source", "type": "datasource", "query": "prometheus",
            "current": {"selected": True, "text": "prometheus", "value": "ced6krlr4hgxsb"},
            "hide": 0, "refresh": 1, "regex": "", "includeAll": False, "multi": False}


def dashboard(uid, title, panels, variables, desc, tags, refresh="30s", time_from="now-24h"):
    return {
        "uid": uid, "title": title, "description": desc, "tags": tags,
        "editable": True, "graphTooltip": 1, "refresh": refresh, "schemaVersion": 39,
        "time": {"from": time_from, "to": "now"}, "timepicker": {},
        "templating": {"list": variables}, "annotations": {"list": [
            {"builtIn": 1, "datasource": {"type": "grafana", "uid": "-- Grafana --"}, "enable": True,
             "hide": True, "iconColor": "rgba(0, 211, 255, 1)", "name": "Annotations & Alerts",
             "type": "dashboard"},
            {"name": "Bot restarts", "datasource": DS, "enable": True, "iconColor": "orange",
             "expr": f"changes(process_start_time_seconds{{{BOT}}}[2m]) > 0", "step": "1m",
             "titleFormat": "ClimaBot restarted", "textFormat": ""},
        ]},
        "links": [], "panels": panels, "fiscalYearStartMonth": 0, "liveNow": False, "weekStart": "",
    }


def rate(metric, labels=""):
    return f"rate({metric}{{{labels}}}[$__rate_interval])"


def inc(metric, labels="", window="$__interval"):
    return f"increase({metric}{{{labels}}}[{window}])"


# ─────────────────────────────────────────────
#  ClimaBot
# ─────────────────────────────────────────────

def climabot():
    L = Layout()
    ok_red = (("red", None), ("green", 1))

    L.row("Overview")
    L.add(stat("Bot", f"up{{{BOT}}}", desc="Whether Prometheus can scrape the bot.", steps=ok_red,
               color_mode="background", graph=False,
               mappings=[{"type": "value", "options": {"0": {"text": "DOWN", "color": "red"},
                                                       "1": {"text": "ONLINE", "color": "green"}}}]), 4, 4)
    L.add(stat("Uptime", f"time() - process_start_time_seconds{{{BOT}}}", unit="dtdurations",
               desc="Time since the bot process started.", graph=False), 4, 4)
    L.add(stat("Servers", "discord_guilds", desc="Discord servers the bot is in."), 4, 4)
    L.add(stat("Gateway latency", "discord_gateway_latency_seconds", unit="s", decimals=3,
               desc="Heartbeat round trip to Discord's gateway.",
               steps=(("green", None), ("orange", 0.25), ("red", 1))), 4, 4)
    L.add(stat("Commands (range)",
               f'sum(increase(discord_command_total{{status="received"}}[$__range]))',
               desc="Slash commands received in the selected time range.", graph=False, decimals=0), 4, 4)
    L.add(stat("Command success (range)",
               'sum(increase(discord_command_total{status="success"}[$__range])) / '
               'sum(increase(discord_command_total{status="received"}[$__range]))',
               unit="percentunit", decimals=1, graph=False,
               desc="Share of commands that completed without an error (unknown locations count as "
                    "successes).",
               steps=(("red", None), ("orange", 0.8), ("green", 0.95))), 4, 4)

    L.add(stat("Response p95 (range)",
               "histogram_quantile(0.95, sum by (le) "
               "(increase(discord_command_response_time_seconds_bucket[$__range])))",
               unit="s", decimals=2, graph=False, desc="95% of commands answered faster than this.",
               steps=(("green", None), ("orange", 3), ("red", 8))), 4, 4)
    L.add(stat("Tracked products", "price_tracked_products", desc="Products being price-tracked."), 4, 4)
    L.add(stat("Overdue price checks", "price_checks_overdue",
               desc="Products whose hourly check is more than 5 minutes late. Should stay at 0.",
               steps=(("green", None), ("orange", 1), ("red", 5))), 4, 4)
    L.add(stat("Bot memory", f"process_resident_memory_bytes{{{BOT}}}", unit="bytes",
               desc="Resident memory of the bot process.",
               steps=(("green", None), ("orange", 200e6), ("red", 500e6))), 4, 4)
    L.add(stat("Bot CPU", f"{rate('process_cpu_seconds_total', BOT)} * 100", unit="percent", decimals=2,
               desc="CPU used by the bot process (100% = one core).",
               steps=(("green", None), ("orange", 50), ("red", 90))), 4, 4)
    L.add(stat("Stack memory", f"sum(container_memory_working_set_bytes{{{STACK}}})", unit="bytes",
               desc="Bot + MongoDB + FlareSolverr + Byparr (working set).",
               steps=(("green", None), ("orange", 2e9), ("red", 3e9))), 4, 4)

    # ── Commands ──────────────────────────────
    L.row("Commands")
    L.add(timeseries("Commands by type",
                     [target(f"sum by (command) ({inc('discord_command_total', 'status=\"received\"')})",
                             "{{command}}")],
                     bars=True, stack=True, legend_calcs=("sum",),
                     desc="Slash commands received per interval."), 12, 8)
    L.add(timeseries("Command errors",
                     [target(f'sum by (command) ({inc("discord_command_total", "status=\"error\"")})',
                             "{{command}}")],
                     bars=True, stack=True, legend_calcs=("sum",),
                     desc="Commands that failed with an error (the user got an error message)."), 12, 8)
    L.add(timeseries("Response time",
                     [target(f"histogram_quantile({q}, sum by (le) "
                             f"({rate('discord_command_response_time_seconds_bucket')}))", f"p{int(q*100)}",
                             ref=r)
                      for q, r in ((0.5, "A"), (0.95, "B"), (0.99, "C"))],
                     unit="s", desc="Time from receiving a command to finishing the reply."), 12, 8)
    L.add(timeseries("Response time p95 by command",
                     [target("histogram_quantile(0.95, sum by (le, command) "
                             f"({rate('discord_command_response_time_seconds_bucket')}))", "{{command}}")],
                     unit="s", desc="Price-tracking adds are slow by design: they may load the page in a "
                                    "browser."), 12, 8)
    L.add(table("Commands in the selected range", [
        ("A", 'sum by (command) (increase(discord_command_total{status="received"}[$__range]))'),
        ("B", 'sum by (command) (increase(discord_command_total{status="success"}[$__range]))'),
        ("C", 'sum by (command) (increase(discord_command_total{status="error"}[$__range]))'),
        ("D", 'sum by (command) (increase(discord_command_total{status="success"}[$__range])) / '
              'sum by (command) (increase(discord_command_total{status="received"}[$__range]))'),
        ("E", "histogram_quantile(0.95, sum by (le, command) "
              "(increase(discord_command_response_time_seconds_bucket[$__range])))"),
    ], rename={"command": "Command", "Value #A": "Received", "Value #B": "Succeeded", "Value #C": "Failed",
               "Value #D": "Success rate", "Value #E": "p95 response"},
        units={"Success rate": "percentunit", "p95 response": "s", "Received": "none", "Succeeded": "none",
               "Failed": "none"},
        sort_by="Received"), 24, 7)

    # ── Weather & roles ───────────────────────
    L.row("Weather & roles")
    L.add(timeseries("Weather API calls",
                     [target(f"sum by (type, status) ({inc('weather_api_requests_total')})",
                             "{{type}} {{status}}")],
                     bars=True, stack=True, legend_calcs=("sum",),
                     desc="OpenCage geocoding and Open-Meteo current/forecast requests."), 8, 8)
    L.add(timeseries("Weather API error rate",
                     [target('sum by (type) (rate(weather_api_requests_total{status="error"}[$__rate_interval])) '
                             '/ sum by (type) (rate(weather_api_requests_total[$__rate_interval]))', "{{type}}")],
                     unit="percentunit", desc="Failed calls to the weather and geocoding APIs."), 8, 8)
    L.add(bargauge("Role changes (range)",
                   'sum by (action) (increase(discord_role_assignments_total[$__range]))', "{{action}}",
                   desc="Self-assigned roles added/removed through /roles, and failures."), 8, 8)

    # ── Price tracker ─────────────────────────
    L.row("Price tracker")
    L.add(timeseries("Tracked products", [
        target("price_tracked_products", "tracked", "A"),
        target("price_checks_overdue", "overdue (>5 min late)", "B"),
    ], legend_calcs=("lastNotNull", "max"),
        overrides=[{"matcher": {"id": "byName", "options": "overdue (>5 min late)"},
                    "properties": [{"id": "color", "value": {"mode": "fixed", "fixedColor": "red"}}]}]), 8, 8)
    L.add(timeseries("Price checks",
                     [target(f"sum by (status) ({inc('price_checks_total')})", "{{status}}")],
                     bars=True, stack=True, legend_calcs=("sum",),
                     desc="Hourly checks: price unchanged, changed (owner gets a DM), or failed.",
                     overrides=[{"matcher": {"id": "byName", "options": n},
                                 "properties": [{"id": "color", "value": {"mode": "fixed", "fixedColor": c}}]}
                                for n, c in (("unchanged", "green"), ("changed", "blue"), ("error", "red"))]),
          8, 8)
    L.add(timeseries("Price check duration",
                     [target(f"histogram_quantile({q}, sum by (le) "
                             f"({rate('price_check_duration_seconds_bucket')}))", f"p{int(q*100)}", ref=r)
                      for q, r in ((0.5, "A"), (0.95, "B"))],
                     unit="s", desc="Time per product, all fetch methods included."), 8, 8)
    L.add(timeseries("Page fetches by method",
                     [target(f"sum by (method, result) ({inc('price_scrape_fetches_total')})",
                             "{{method}} · {{result}}")],
                     bars=True, stack=True, legend_calcs=("sum",),
                     desc="Every fetch attempt: direct HTTP, FlareSolverr, Byparr, Mercado Livre API. "
                          "'blocked' = bot protection page, 'no_price' = page loaded without a price."), 12, 8)
    L.add(bargauge("Fetch success rate by method (range)",
                   'sum by (method) (increase(price_scrape_fetches_total{result="success"}[$__range])) / '
                   'sum by (method) (increase(price_scrape_fetches_total[$__range]))',
                   "{{method}}", unit="percentunit", max_=1,
                   steps=(("red", None), ("orange", 0.5), ("green", 0.8)),
                   desc="Share of attempts per method that returned a price. A failed direct fetch "
                        "is normal: the browsers take over."), 12, 8)

    # ── Resources ─────────────────────────────
    L.row("Resources")
    L.add(timeseries("CPU by container",
                     [target(f"sum by ({SVC}) ({rate('container_cpu_usage_seconds_total', STACK)}) * 100",
                             f"{{{{{SVC}}}}}")],
                     unit="percent", desc="100% = one full CPU core."), 8, 8)
    L.add(timeseries("Memory by container",
                     [target(f"sum by ({SVC}) (container_memory_working_set_bytes{{{STACK}}})",
                             f"{{{{{SVC}}}}}")],
                     unit="bytes", legend_calcs=("lastNotNull", "max"), stack=True,
                     desc="Working set (what the kernel counts against limits)."), 8, 8)
    L.add(bargauge("Memory vs limit",
                   f"sum by ({SVC}) (container_memory_working_set_bytes{{{STACK}}}) / "
                   f"sum by ({SVC}) (container_spec_memory_limit_bytes{{{STACK}}} > 0)",
                   f"{{{{{SVC}}}}}", unit="percentunit", max_=1,
                   steps=(("green", None), ("orange", 0.75), ("red", 0.9)),
                   desc="Containers with a memory limit (the browsers, 1 GB each). Near 100% they get "
                        "OOM-killed and restarted."), 8, 8)
    L.add(timeseries("Network by container", [
        target(f"sum by ({SVC}) ({rate('container_network_receive_bytes_total', STACK)})",
               f"{{{{{SVC}}}}} in", "A"),
        target(f"-sum by ({SVC}) ({rate('container_network_transmit_bytes_total', STACK)})",
               f"{{{{{SVC}}}}} out", "B"),
    ], unit="Bps", min0=False, desc="Received (up) and sent (down)."), 8, 8)
    L.add(timeseries("Disk I/O by container", [
        target(f"sum by ({SVC}) ({rate('container_fs_reads_bytes_total', STACK)})", f"{{{{{SVC}}}}} read", "A"),
        target(f"-sum by ({SVC}) ({rate('container_fs_writes_bytes_total', STACK)})",
               f"{{{{{SVC}}}}} write", "B"),
    ], unit="Bps", min0=False, desc="Read (up) and written (down)."), 8, 8)
    L.add(timeseries("Bot process", [
        target(f"process_threads{{{BOT}}}", "threads", "A"),
        target(f"process_open_fds{{{BOT}}}", "open files/sockets", "B"),
        target("discord_gateway_latency_seconds * 1000", "gateway latency (ms)", "C"),
    ], legend_calcs=("lastNotNull", "max"), desc="Leaks show up here as steadily growing lines."), 8, 8)
    L.add(gauge("Host CPU",
                'sum(rate(container_cpu_usage_seconds_total{id="/"}[$__rate_interval])) / '
                'max(machine_cpu_cores)', desc="Whole server."), 8, 6)
    L.add(gauge("Host memory", 'max(container_memory_working_set_bytes{id="/"}) / max(machine_memory_bytes)',
                desc="Whole server, working set."), 8, 6)
    L.add(gauge("Fullest disk", f"max(container_fs_usage_bytes{{{DISKS}}} / container_fs_limit_bytes{{{DISKS}}})",
                desc="Most-used real disk on the server."), 8, 6)
    # ── Logs (Loki) ───────────────────────────
    L.row("Logs")
    # The message becomes the log line; the other fields (url, error, …) show
    # in each line's details.
    message = '| json | line_format "{{.fields_message}}"'
    volume = timeseries("Log lines by level",
                        [loki_target(f"sum by (level) (count_over_time({LOGS} [$__interval]))", "{{level}}")],
                        bars=True, stack=True, legend_calcs=("sum",),
                        desc="Spikes of warn/error lines usually explain an alert.",
                        overrides=[{"matcher": {"id": "byName", "options": n},
                                    "properties": [{"id": "color", "value": {"mode": "fixed", "fixedColor": c}}]}
                                   for n, c in (("info", "green"), ("warn", "orange"), ("error", "red"),
                                                ("debug", "blue"))])
    volume["datasource"] = LOKI
    L.add(volume, 24, 6)
    L.add(logs("Warnings and errors", f'{{job="climabot", level=~"warn|error"}} {message}',
               desc="Newest first. Expand a line for its fields."), 24, 10)
    L.add(logs("All logs", f"{LOGS} {message}",
               desc="Everything the bot logged (info and above). On the server the same JSON is in "
                    "/var/log/climabot/climabot.log.YYYY-MM-DD (7 days kept); Loki keeps 30 days."), 24, 12)

    return dashboard(
        "ded6lbvrb7lkwc", "ClimaBot", L.panels, [datasource_var()],
        "ClimaBot Discord bot: commands, weather APIs, roles, price tracker and resource usage.",
        ["climabot", "discord"])


# ─────────────────────────────────────────────
#  Docker host & containers
# ─────────────────────────────────────────────

def docker_host():
    L = Layout()
    sel = f'{PROJ}=~"$project"'
    legend = f"{{{{{PROJ}}}}}/{{{{{SVC}}}}}"
    by = f"{PROJ}, {SVC}"

    L.row("Host")
    L.add(gauge("CPU", 'sum(rate(container_cpu_usage_seconds_total{id="/"}[$__rate_interval])) / '
                       'max(machine_cpu_cores)'), 6, 6)
    L.add(gauge("Memory", 'max(container_memory_working_set_bytes{id="/"}) / max(machine_memory_bytes)'), 6, 6)
    L.add(gauge("Fullest disk",
                f"max(container_fs_usage_bytes{{{DISKS}}} / container_fs_limit_bytes{{{DISKS}}})"), 6, 6)
    L.add(stat("Running containers", f"count(count by (name) (container_last_seen{{{sel}}}))",
               desc="Containers seen by cAdvisor in the selected projects."), 6, 6)
    L.add(timeseries("Host CPU", [target('sum(rate(container_cpu_usage_seconds_total{id="/"}[$__rate_interval])) '
                                         '/ max(machine_cpu_cores)', "CPU")], unit="percentunit"), 8, 7)
    L.add(timeseries("Host memory", [
        target('max(container_memory_working_set_bytes{id="/"})', "used (working set)", "A"),
        target("max(machine_memory_bytes)", "total", "B"),
    ], unit="bytes", legend_calcs=("lastNotNull", "max")), 8, 7)
    L.add(timeseries("Disk usage", [
        target(f"container_fs_usage_bytes{{{DISKS}}} / container_fs_limit_bytes{{{DISKS}}}", "{{device}}"),
    ], unit="percentunit", legend_calcs=("lastNotNull",)), 8, 7)

    L.row("Containers")
    L.add(timeseries("CPU", [target(f"sum by ({by}) ({rate('container_cpu_usage_seconds_total', sel)}) * 100",
                                    legend)], unit="percent", desc="100% = one core."), 12, 9)
    L.add(timeseries("Memory (working set)", [target(f"sum by ({by}) (container_memory_working_set_bytes{{{sel}}})",
                                                     legend)], unit="bytes",
                     legend_calcs=("lastNotNull", "max")), 12, 9)
    L.add(timeseries("Network", [
        target(f"sum by ({by}) ({rate('container_network_receive_bytes_total', sel)})", legend + " in", "A"),
        target(f"-sum by ({by}) ({rate('container_network_transmit_bytes_total', sel)})", legend + " out", "B"),
    ], unit="Bps", min0=False, desc="Received (up) and sent (down)."), 12, 9)
    L.add(timeseries("Disk I/O", [
        target(f"sum by ({by}) ({rate('container_fs_reads_bytes_total', sel)})", legend + " read", "A"),
        target(f"-sum by ({by}) ({rate('container_fs_writes_bytes_total', sel)})", legend + " write", "B"),
    ], unit="Bps", min0=False, desc="Read (up) and written (down)."), 12, 9)
    L.add(table("Containers now", [
        ("A", f"sum by ({by}) ({rate('container_cpu_usage_seconds_total', sel)}) * 100"),
        ("B", f"sum by ({by}) (container_memory_working_set_bytes{{{sel}}})"),
        ("C", f"sum by ({by}) (container_spec_memory_limit_bytes{{{sel}}} > 0)"),
        ("D", f"sum by ({by}) (container_fs_usage_bytes{{{sel}}})"),
    ], rename={PROJ: "Project", SVC: "Service", "Value #A": "CPU", "Value #B": "Memory",
               "Value #C": "Memory limit", "Value #D": "Writable layer"},
        units={"CPU": "percent", "Memory": "bytes", "Memory limit": "bytes", "Writable layer": "bytes"},
        sort_by="Memory"), 24, 10)

    project_var = {
        "name": "project", "label": "Compose project", "type": "query", "datasource": DS,
        "query": {"query": f"label_values(container_last_seen, {PROJ})", "refId": "project"},
        "definition": f"label_values(container_last_seen, {PROJ})",
        "refresh": 2, "includeAll": True, "multi": True, "allValue": ".*",
        "current": {"selected": True, "text": ["All"], "value": ["$__all"]}, "sort": 1, "hide": 0,
    }
    return dashboard("docker-host", "Docker Host & Containers", L.panels, [datasource_var(), project_var],
                     "Server and per-container CPU, memory, disk and network (cAdvisor).",
                     ["docker", "infrastructure"], time_from="now-6h")


if __name__ == "__main__":
    for name, build in (("climabot.json", climabot), ("docker-host.json", docker_host)):
        (OUT / name).write_text(json.dumps(build(), indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print("wrote", OUT / name)
