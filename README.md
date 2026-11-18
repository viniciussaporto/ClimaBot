<a name="readme-top"></a>

<div align="center">

# 🌦️ ClimaBot

**A Discord bot for weather, football scores, self-assignable roles and product price tracking.**

[![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)](https://www.rust-lang.org/)
[![Discord](https://img.shields.io/badge/Discord-slash%20commands-5865F2?logo=discord&logoColor=white)](https://discord.com/api/oauth2/authorize?client_id=1109598244884979792&permissions=534723951680&scope=applications.commands%20bot)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE.md)

[**Add it to your server**](https://discord.com/api/oauth2/authorize?client_id=1109598244884979792&permissions=534723951680&scope=applications.commands%20bot)
· [Report a bug](https://github.com/viniciussaporto/ClimaBot/issues)
· [Request a feature](https://github.com/viniciussaporto/ClimaBot/issues)

<img src="https://i.imgur.com/wbJhigJ.png" alt="ClimaBot weather embed" />

</div>

<details>
  <summary><b>Contents</b></summary>

- [Features](#features)
- [Commands](#commands)
- [Football](#football)
- [Price tracking](#price-tracking)
- [Self-hosting](#self-hosting)
- [Configuration](#configuration)
- [Operations](#operations)
- [Development](#development)
- [Contributing](#contributing)
- [License and contact](#license-and-contact)

</details>

## Features

- **Weather**: current conditions, an animated precipitation map (now and the next 3 hours) and a 5-day forecast for any place on Earth, in metric or imperial units. Data from [Open-Meteo](https://open-meteo.com), geocoding from [OpenCage](https://opencagedata.com), map tiles from [OpenStreetMap](https://www.openstreetmap.org/copyright).
- **Football**: live scores, fixtures, results, tables, top scorers, match events, line-ups, teams, squads and players for the Brasileirão, Copa do Brasil, Libertadores, Sudamericana, Brazilian state championships and the main European leagues and cups. Data [Powered by SportScore](https://sportscore.com/), with [football-data.org](https://www.football-data.org/) as a backup and squads from [API-Football](https://www.api-football.com/).
- **Self-assignable roles**: a menu members use to pick their own roles. Roles with moderation or admin permissions, managed roles and roles above the bot are never offered.
- **Price tracking**: watch product pages in any currency and get a DM when the price changes, with a one-click **❌ Stop tracking** button.
- **Built to run unattended**: Prometheus metrics, a `/status` health endpoint, Grafana dashboards and alerts as code, JSON logs shipped to Loki, and a nightly self-update.

## Commands

| Command | What it does |
| --- | --- |
| `/weather <location> [units]` | Current temperature, feels-like, humidity, clouds, wind and pressure, with an animated map of rain, snow and thunderstorms for now and the next 3 hours |
| `/forecast <location> [units]` | 5-day forecast: highs, lows and chance of rain (dates shown day-first, e.g. `Tue, 29/09`) |
| `/roles` | Menu to toggle self-assignable roles (server only) |
| `/pt add <url>` | Start tracking a product page |
| `/pt list` | Your tracked products, current prices and next check time |
| `/pt history <item>` | Price history of a product, with lowest and highest price |
| `/pt remove <item>` | Stop tracking a product |
| `/live all [competition]` | Live matches in the followed competitions, by competition |
| `/live match <match>` | One live match: score, status, clock and latest events |
| `/live competition <competition>` | Live matches in one competition |
| `/football competition list` | The competitions followed and what's available for each |
| `/football competition fixtures <competition> [date]` | Matches and results on a day (`today`, `tomorrow`, `yesterday` or `YYYY-MM-DD`) |
| `/football competition table <competition>` | Standings (group tables for competitions with groups) |
| `/football competition scorers <competition> [stat]` | Top goalscorers or assists |
| `/football match details\|events\|lineup\|stats <match>` | Score, key events, the full timeline, line-ups and formations, team statistics |
| `/football team info\|squad\|fixtures <team>` | Standings, form, recent and next matches; the current squad; all fixtures |
| `/football team stats <team> <competition>` | A team's record in a competition |
| `/football player <name>` | A player's season statistics |
| `/help` | Command overview |

`units` is optional: **Metric** (°C, km/h, hPa) by default, or **Imperial** (°F, mph, inHg). For `/pt history` and `/pt remove`, `item` is the position shown by `/pt list` or the product URL.

## Football

`/live` shows the live scoreboard, refreshed about once a minute while a followed match is about to start, under way or just ended. `/football` shows cached data and says how old it is; it's never presented as live. Every reply names its source.

**Competitions:** Brasileirão Séries A, B and C, Copa do Brasil, Supercopa do Brasil, Copa do Nordeste, Copa Verde, Paulistão, Carioca, Mineiro, Gaúcho, Copa Libertadores, Copa Sudamericana, the Champions, Europa and Conference Leagues, and the leagues and cups of England, Spain, Germany, Italy, France and Portugal (`src/football/competitions.rs`). Options autocomplete as you type: competitions, matches of yesterday to tomorrow, teams in the followed tables, top scorers.

**How the data is fetched.** A scheduler is the only thing that polls the providers: fixture lists every 6 hours, results and tables daily, the live scoreboard each minute during matches, and match details every 2.5 minutes for live matches in the ⭐ competitions (`FOOTBALL_PRIORITY`) or that someone asked about. When a match leaves the live list, its final score, fixture list and table are refreshed. Commands read the cache (memory, then MongoDB `football_cache`); if what they need is missing or too old they trigger one shared refresh, however many people ask. Each provider has a daily budget, restored after restarts and corrected from the provider's own quota headers; as it runs low, optional refreshes stop first and the live scoreboard last. When a provider fails or runs out, replies show the newest cached copy with its age, or say that nothing is available. No data is made up.

| Source | Used for | Free limits |
| --- | --- | --- |
| [SportScore](https://sportscore.com/developers/) | Everything, including live scores | ~10,000 requests/day per IP; attribution required |
| [football-data.org](https://www.football-data.org/) (`FOOTBALL_DATA_TOKEN`) | Backup for fixtures, tables and scorers of its 13 free competitions; its scores are delayed and never used for `/live` | 10 requests/minute |
| [API-Football](https://www.api-football.com/) (`API_FOOTBALL_KEY`) | Squads (its free plan can't read current-season fixtures) | 100 requests/day |

Match statistics and some player statistics are often missing from the free sources; replies then say they aren't provided rather than showing empty tables.

## Price tracking

Each user can track up to 25 products. Every product is checked about once an hour, at a random time within ±15 minutes of its hourly slot, with random gaps between requests to the same shop, so the traffic doesn't look scheduled.

When the price changes, ClimaBot DMs the owner with the old and new price and the change in percent. If a page can't be read for 24 hours in a row, it sends a one-time warning instead. Both messages carry a **❌ Stop tracking** button that removes the product on the spot.

**Tested shops:** KaBuM, Amazon Brasil, Magazine Luiza, Pichau, Terabyte and AliExpress. Other shops may work, but that isn't guaranteed; Mercado Livre needs one of the options below.

**How pages are fetched.** A direct request is tried first, then two challenge-solving browsers that also render JavaScript:

1. [FlareSolverr](https://github.com/FlareSolverr/FlareSolverr) (Chrome): handles Akamai and many Cloudflare pages.
2. [Byparr](https://github.com/ThePhaseless/Byparr) (Firefox): handles Cloudflare challenges FlareSolverr can't.

The method that last worked for a shop is tried first next time and is remembered in MongoDB across restarts. When `/pt add` can't get a price from a page, the bot logs a `No price found when adding a product` warning and adds the shop to the `failed_sites` collection (attempts, last URL, last reason), a list of shops to look into later: see `climabot failed-sites`. Prices are read from schema.org data, product meta tags, dedicated rules for Amazon and AliExpress, or common price markup.

**Shops that block servers.** Some shops refuse datacenter IPs whatever the browser does (Mercado Livre's website, for example). For those, set `SCRAPER_PROXY` to a residential proxy. For Mercado Livre specifically, you can instead set `ML_CLIENT_ID`/`ML_CLIENT_SECRET` from a free [Mercado Livre developer app](https://developers.mercadolivre.com.br) to use the official API. Shops that adapt to the visitor's country (AliExpress) show prices for the server's or proxy's country.

## Self-hosting

ClimaBot is written in Rust and ships as a Docker Compose stack: the bot, MongoDB, FlareSolverr and Byparr.

1. Create a bot in the [Discord Developer Portal](https://discord.com/developers/applications). Invite it with the `bot` and `applications.commands` scopes and the **Manage Roles** permission (needed for `/roles`). No privileged gateway intents are required.
2. Get a free API key from [OpenCage](https://opencagedata.com).
3. Copy `.env.example` to `.env` and fill in `DISCORD_TOKEN` and `OPENCAGEAPIKEY`.
4. Create the network shared with your monitoring stack (once), and attach Prometheus to it:
   ```sh
   docker network create climabot-monitoring
   ```
5. Start everything:
   ```sh
   docker compose up -d --build
   ```
   MongoDB sits on an internal network only the bot can reach.

Then check the deployment: Discord credentials, registered commands, role menus per server, the weather and football APIs and the database.

```sh
docker compose exec climabot climabot selfcheck [product-url ...]
```

## Configuration

Everything is set in `.env`; `.env.example` documents every option.

| Variable | Default | Purpose |
| --- | --- | --- |
| `DISCORD_TOKEN` | *(required)* | Bot token |
| `OPENCAGEAPIKEY` | *(required)* | Geocoding API key |
| `MONGODB_URI` / `MONGODB_DB` | set by compose | Database for price tracking |
| `FLARESOLVERR_URL` / `BYPARR_URL` | set by compose | Challenge-solving browsers; unset = not used |
| `FLARESOLVERR_CONCURRENCY` / `BYPARR_CONCURRENCY` | `1` | Browser instances each may run at once |
| `FLARESOLVERR_MEM_LIMIT` / `BYPARR_MEM_LIMIT` | `1536m` / `2g` | Browser memory caps (compose) |
| `SCRAPER_PROXY` | – | HTTP or SOCKS5 proxy for all scraping |
| `ML_CLIENT_ID` / `ML_CLIENT_SECRET` | – | Mercado Livre API app |
| `LOG_DIR` / `CLIMABOT_LOG_DIR` | `/var/log/climabot` | JSON logs, rotated daily, 7 days kept |
| `SPORTSCORE_API_KEY` | – | Registered SportScore key (raises their rate limit; works without one) |
| `FOOTBALL_DATA_TOKEN` | – | football-data.org token: backup source for fixtures, tables, scorers |
| `API_FOOTBALL_KEY` | – | API-Football key: squads (`/football team squad`) |
| `FOOTBALL_PRIORITY` | ⭐ in `competitions.rs` | Comma-separated competition keys whose live match details are polled |
| `SPORTSCORE_DAILY_BUDGET` / `API_FOOTBALL_DAILY_BUDGET` | `9000` / `100` | Requests per UTC day the bot allows itself |
| `RADAR_RAIN_CHANCE` | `20` | `/weather` map: fetch the precipitation grid (~108 Open-Meteo calls) only when the chance of precipitation at the location reaches this % within 3 hours |
| `METRICS_PORT` / `METRICS_HOST_PORT` | `9464` / `9465` | Port for `/metrics` and `/status` (host side bound to `127.0.0.1`) |
| `RUST_LOG` | `info` | Log level |

## Operations

### Branches and deploys

| Branch | Role |
| --- | --- |
| `main` | Production. The server deploys it on every update and nightly restart. |
| `preprod` | Changes land here first and reach production through a pull request into `main`. |

Older versions are kept as the tags `archive/DeployThis` (the first Rust deployment branch) and `archive/javascript` (the retired TypeScript bot).

### The `climabot` command

`deploy/install.sh` installs a `climabot` command on the server, plus `climabot-*` shortcuts and an SSH login banner listing them. The checkout lives in `/root/climabot-rust` and follows `main`.

| Alias | Command | What it does |
| --- | --- | --- |
| `climabot-start` | `climabot start` | Pull `main`, rebuild the bot, pull the MongoDB and browser images, start the stack |
| `climabot-update` | `climabot update` | Pull `main`, rebuild and restart the running bot |
| `climabot-stop` | `climabot stop` | Stop the bot and its stack |
| `climabot-status` | `climabot status` | Branch, commit and container status |
| `climabot-logs` | `climabot logs` | Follow the bot's logs |
| `climabot-failed-sites` | `climabot failed-sites` | Shops where `/pt add` couldn't find a price, most recent first |
| `climabot-help` | | Show the list again |

`climabot update` also runs **nightly at 05:30 UTC** (`climabot-update.timer`, the quietest hour for the bot's users) and **3 minutes after every boot**, so a restarted server always comes back on the latest `main`. ClimaBot alerts are muted from 05:25 to 06:15 UTC for the nightly run.

### Monitoring

- **Metrics**: Prometheus scrapes `climabot:9464/metrics` over the shared network.
- **Dashboards**: generated from `deploy/grafana/build_dashboards.py`. Run it, commit the JSON, and `climabot update` provisions them (folders **ClimaBot** and **Infrastructure Metrics**). Container and host metrics come from cAdvisor.
- **Alerts**: generated from `deploy/grafana/build_alerts.py`.
  - `alerting/urgent.yaml`: bot, website, API, status page, certificates, disk and memory. Sent to **Telegram**.
  - `alerting/info.yaml`: restarts, error rates, price checks, slow site and resources. Sent to **Discord**, through a webhook URL stored in `/root/monitoring/secrets/discord-webhook-url`; without it only the urgent alerts are installed.
- **Probes**: the blackbox exporter checks website uptime and TLS expiry (`deploy/monitoring/blackbox.yml`).
- **Logs**: the website stack's Alloy ships `/var/log/climabot/climabot.log.*` to Loki as `{job="climabot", level="info|warn|error"}`, kept for 30 days. Query them in Explore, e.g. `{job="climabot", level="error"} | json`, or in the dashboard's **Logs** row.

### When Grafana itself is down

All alerts above come from Grafana, so if Grafana or the whole server dies, nothing is sent. Two monitors on an external uptime service (UptimeRobot, Better Stack or similar; free plans are enough) cover that, notifying the same Telegram chat:

| Monitor | URL | Healthy when |
| --- | --- | --- |
| ClimaBot | `https://vinisaporto.de/api/climabot/status` | HTTP 200 and the body contains `"ok":true` |
| Grafana | `https://grafana.vinisaporto.de/api/health` | HTTP 200 and the body contains `"database": "ok"` |

`/status` returns `{"ok":true,"discord":true,"database":true,"version":"…"}` when a shard is connected to Discord and MongoDB answers. Otherwise, and for about a minute after a start, it returns HTTP 503 with `"ok":false`. Caddy forwards only that path to the bot (website repo, `deploy/caddy/vinisaporto.caddy`); `/metrics` stays private. Check every 5 minutes, alert after 2 failures, and add a maintenance window for the nightly update if the service supports one.

## Development

```sh
cargo test     # unit tests
cargo run      # runs the bot, reading .env
```

To debug price extraction for a shop, save the product page and run:

```sh
climabot extract page.html <url>
```

The Docker image builds with OpenSSL (native-tls) and runs as an unprivileged user (uid 10001).

## Contributing

Suggestions and pull requests are welcome. Open an [issue](https://github.com/viniciussaporto/ClimaBot/issues) with the `enhancement` tag, or:

1. Fork the project.
2. Create a branch from `preprod` (`git checkout -b feature/my-feature preprod`).
3. Commit your changes and push the branch.
4. Open a pull request into `preprod`.

## License and contact

Distributed under the MIT License; see [LICENSE.md](LICENSE.md).

Vinicius Saporto · [@viniciussaporto](https://twitter.com/viniciussaporto) · vinicius.saporto@gmail.com

<p align="right">(<a href="#readme-top">back to top</a>)</p>
