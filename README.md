<a name="readme-top"></a>
<!--
*** Thanks for checking out the Best-README-Template. If you have a suggestion
*** that would make this better, please fork the repo and create a pull request
*** or simply open an issue with the tag "enhancement".
*** Don't forget to give the project a star!
*** Thanks again! Now go create something AMAZING! :D
-->



<!-- PROJECT SHIELDS -->
<!--
*** I'm using markdown "reference style" links for readability.
*** Reference links are enclosed in brackets [ ] instead of parentheses ( ).
*** See the bottom of this document for the declaration of the reference variables
*** for contributors-url, forks-url, etc. This is an optional, concise syntax you may use.
*** https://www.markdownguide.org/basic-syntax/#reference-style-links
-->
<!-- [![Contributors][contributors-shield]][https://github.com/viniciussaporto] -->
<!-- [![Forks][forks-shield]][forks-url] -->
<!-- [![Stargazers][stars-shield]][stars-url] -->
<!-- [![Issues][issues-shield]][issues-url] -->
<!-- [![MIT License][license-shield]][https://opensource.org/license/mit/] -->
<!-- [![LinkedIn][linkedin-shield]][https://www.linkedin.com/in/vinicius-saporto/] -->



<!-- PROJECT LOGO -->
<br />
<div align="center">
  <a href="https://github.com/viniciussaporto/ClimaBot">
    <!-- <img src="images/logo.png" alt="Logo" width="80" height="80"> -->
  </a>

<h3 align="center">ClimaBot</h3>

  <p align="center">
    This Discord bot retrieves climate information on Open-Meteo API and Geocoding information at OpenCage API and presents using slash commands (/weather location) on the server it has been added. This is hosted by me, but feel free to host it yourself, because it has API request limits on part of OpenCageAPI
    <br />
    <a href="https://github.com/viniciussaporto/ClimaBot"><strong>Explore the docs »</strong></a>
    <br />
    <br />
    <a href="https://discord.com/api/oauth2/authorize?client_id=1109598244884979792&permissions=534723951680&scope=applications.commands%20bot">View Demo</a>
    ·
    <a href="https://github.com/viniciussaporto/ClimaBot/issues">Report Bug</a>
    ·
    <a href="https://github.com/viniciussaporto/ClimaBot/issues">Request Feature</a>
  </p>
</div>



<!-- TABLE OF CONTENTS -->
<details>
  <summary>Table of Contents</summary>
  <ol>
    <li>
      <a href="#about-the-project">About The Project</a>
      <ul>
        <li><a href="#built-with">Built With</a></li>
      </ul>
    </li>
    <li>
      <a href="#getting-started">Getting Started</a>
      <ul>
        <li><a href="#prerequisites">Prerequisites</a></li>
        <li><a href="#installation">Installation</a></li>
      </ul>
    </li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#roadmap">Roadmap</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>



<!-- ABOUT THE PROJECT -->
## About The Project
<p align="center">
  <img src="https://i.imgur.com/wbJhigJ.png" />
</p>

Well, to use it, it's as simple as clicking the `View Demo` button at the start of this page and adding it to your Discord server.

If you want to self-host (which is what I recommend), ClimaBot is written in Rust and ships as a Docker image:

1. Create a bot at the [Discord Developer Portal](https://discord.com/developers/applications) and invite it with the `bot` and `applications.commands` scopes and the **Manage Roles** permission (needed for `/roles`). No privileged gateway intents are required.
2. Get a free API key from [OpenCage](https://opencagedata.com).
3. Copy `.env.example` to `.env` and fill in `DISCORD_TOKEN` and `OPENCAGEAPIKEY`.
4. Create the network shared with your monitoring stack (once) with `docker network create climabot-monitoring`, and attach Prometheus to it.
5. Run `docker compose up -d --build`. This starts the bot and a MongoDB instance on an internal network only the bot can reach.

Prometheus scrapes `climabot:9464/metrics` over the shared network (it is also published on `127.0.0.1:9465`). The same port serves `/status`, a small JSON health report for uptime monitors (see below). Price-tracking data lives in MongoDB (`products`, `price_history` and `fetch_methods` collections), and JSON logs rotate daily under `/var/log/climabot` on the host (7 days kept).

To verify a deployment (Discord credentials, registered commands, per-server role menus, weather APIs, database) run:

```sh
docker compose exec climabot climabot selfcheck [product-url ...]
```

For local development: `cargo test` and `cargo run` (reads `.env`).

#### Managing the production server

`deploy/install.sh` installs a `climabot` command, plus `climabot-*` shortcuts and an SSH login banner listing them (`/etc/profile.d/climabot.sh`). The server's checkout (`/root/climabot-rust`) follows the `main` branch; changes land on `preprod` first and are merged into `main` to deploy them:

| Alias | Command | What it does |
| --- | --- | --- |
| `climabot-start` | `climabot start` | `git pull` + rebuild the bot (and pull MongoDB/browser images), start the stack |
| `climabot-update` | `climabot update` | `git pull` + rebuild + restart the running bot. Also runs **nightly at 05:30 UTC** (`climabot-update.timer`, the quietest hour for the bot's users); ClimaBot alerts are muted 05:25–06:15 UTC for it |
| `climabot-stop` | `climabot stop` | Stop the bot and its stack |
| `climabot-status` | `climabot status` | Show what is running |
| `climabot-logs` | `climabot logs` | Follow the running bot's logs |
| `climabot-help` | | Show the command list (also printed on every SSH login, with which bot is running) |

Grafana dashboards live in `deploy/grafana/` as code: edit `build_dashboards.py`, run it to regenerate the JSON, and commit. `climabot update` provisions them into the monitoring stack (folders **ClimaBot** and **Infrastructure Metrics**). Container and host metrics come from cAdvisor in the monitoring stack.

Alerts are also code: `deploy/grafana/build_alerts.py` generates `alerting/urgent.yaml` (bot, website, API, status page, certificates, disk/memory: sent to the **Telegram** contact point) and `alerting/info.yaml` (restarts, error rates, price checks, slow site, resources: sent to **Discord**). The Discord contact point is created from a webhook URL stored in `/root/monitoring/secrets/discord-webhook-url`; without it only the urgent alerts are installed. Website uptime and TLS expiry are probed by the blackbox exporter (`deploy/monitoring/blackbox.yml`).

**Logs** are in Grafana too: the website stack's Alloy tails `/var/log/climabot/climabot.log.*` and ships it to its Loki as `{job="climabot", level="info|warn|error"}` (30 days kept). The ClimaBot dashboard's **Logs** row shows log volume by level, warnings/errors and everything else; in Explore, query e.g. `{job="climabot", level="error"} | json`. The pipeline lives in the website repo (`deploy/alloy/config.alloy`).

#### Alerts when Grafana is down

Every alert above is sent by Grafana, so if Grafana (or the whole server) dies, nothing is sent. Two monitors on an external uptime service (UptimeRobot, Better Stack or similar; free plans are enough) cover that, notifying the same Telegram chat as the urgent alerts:

| Monitor | URL | Healthy when |
| --- | --- | --- |
| ClimaBot | `https://vinisaporto.de/api/climabot/status` | HTTP 200 and the body contains `"ok":true` |
| Grafana | `https://grafana.vinisaporto.de/api/health` | HTTP 200 and the body contains `"database": "ok"` |

`/status` answers `{"ok":true,"discord":true,"database":true,"version":"…"}` when a shard is connected to Discord's gateway and MongoDB answers, and HTTP 503 with `"ok":false` otherwise (also for a minute or so after a start). Caddy forwards only that path to the bot (website repo, `deploy/caddy/vinisaporto.caddy`); `/metrics` stays private. Check every 5 minutes, alert after 2 failures, and add a maintenance window for the nightly update (05:25–06:15 UTC) if the service supports it.
<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- ### Built With

* [![Next][Next.js]][Next-url]
* [![React][React.js]][React-url]
* [![Vue][Vue.js]][Vue-url]
* [![Angular][Angular.io]][Angular-url]
* [![Svelte][Svelte.dev]][Svelte-url]
* [![Laravel][Laravel.com]][Laravel-url]
* [![Bootstrap][Bootstrap.com]][Bootstrap-url]
* [![JQuery][JQuery.com]][JQuery-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p> -->



<!-- GETTING STARTED -->
<!-- ## Getting Started

This is an example of how you may give instructions on setting up your project locally.
To get a local copy up and running follow these simple example steps.

### Prerequisites

This is an example of how to list things you need to use the software and how to install them.
* npm
  ```sh
  npm install npm@latest -g
  ```

### Installation

1. Get a free API Key at [https://example.com](https://example.com)
2. Clone the repo
   ```sh
   git clone git@github.com:viniciussaporto/ClimaBot.git
   ```
3. Install NPM packages
   ```sh
   npm install
   ```
4. Enter your API in `config.js`
   ```js
   const API_KEY = 'ENTER YOUR API';
   ```

<p align="right">(<a href="#readme-top">back to top</a>)</p> -->



<!-- USAGE EXAMPLES -->
## Usage

| Command | What it does |
| --- | --- |
| `/weather <location> [units]` | Current conditions for a location; `units: Imperial` shows °F, mph and inHg |
| `/forecast <location> [units]` | 5-day forecast; `units: Imperial` shows °F |
| `/roles` | Menu to toggle self-assignable roles (roles with moderation/admin permissions, managed roles and roles above the bot are never offered) |
| `/pt add <url>` | Track a product page's price in any currency; re-checked about every hour (at a random time within ±15 minutes of its hourly slot, with random gaps between requests to the same shop), with a DM when it changes |
| `/pt list` / `/pt history <item>` / `/pt remove <item>` | Manage tracked products (`item` is the position from `/pt list` or the URL) |
| `/help` | Command overview |

Price tracking tries a direct request first, then two challenge-solving browsers that also render JavaScript: [FlareSolverr](https://github.com/FlareSolverr/FlareSolverr) (Chrome; handles Akamai and many Cloudflare pages) and [Byparr](https://github.com/ThePhaseless/Byparr) (Firefox; handles Cloudflare challenges FlareSolverr can't). The method that last worked for a shop is tried first on the next check, and is remembered in MongoDB across restarts. Prices are read from schema.org data, product meta tags, dedicated rules for Amazon and AliExpress, or common price markup, in any currency.

Some shops block datacenter IPs outright, whatever the browser does (Mercado Livre's website, for example). For those, set `SCRAPER_PROXY` to a residential proxy, or, for Mercado Livre specifically, set `ML_CLIENT_ID`/`ML_CLIENT_SECRET` from a free [Mercado Livre developer app](https://developers.mercadolivre.com.br) to use their official API. Shops that adapt to the visitor's country (AliExpress) show prices for the server's or proxy's country.

To debug a shop: save the page and run `climabot extract page.html <url>`.

<!-- _For more examples, please refer to the [Documentation](https://example.com)_ -->

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- ROADMAP -->
<!-- ## Roadmap

- [ ] Feature 1
- [ ] Feature 2
- [ ] Feature 3
    - [ ] Nested Feature

See the [open issues](https://github.com/github_username/repo_name/issues) for a full list of proposed features (and known issues).

<p align="right">(<a href="#readme-top">back to top</a>)</p> -->



<!-- CONTRIBUTING -->
## Contributing

Contributions are what make the open source community such an amazing place to learn, inspire, and create. Any contributions you make are **greatly appreciated**.

If you have a suggestion that would make this better, please fork the repo and create a pull request. You can also simply open an issue with the tag "enhancement".
Don't forget to give the project a star! Thanks again!

1. Fork the Project
2. Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3. Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the Branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- LICENSE -->
## License

Distributed under the MIT License.

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- CONTACT -->
## Contact

[@viniciussaporto](https://twitter.com/viniciussaporto) - vinicius.saporto@gmail.com

Project Link: [https://github.com/viniciussaporto/ClimaBot](https://github.com/viniciussaporto/ClimaBot)

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- ACKNOWLEDGMENTS -->
<!-- ## Acknowledgments

* []()
* []()
* []() -->

<!-- <p align="right">(<a href="#readme-top">back to top</a>)</p> -->



<!-- MARKDOWN LINKS & IMAGES -->
<!-- https://www.markdownguide.org/basic-syntax/#reference-style-links -->
[contributors-shield]: https://img.shields.io/github/contributors/github_username/repo_name.svg?style=for-the-badge
[contributors-url]: https://github.com/github_username/repo_name/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/github_username/repo_name.svg?style=for-the-badge
[forks-url]: https://github.com/github_username/repo_name/network/members
[stars-shield]: https://img.shields.io/github/stars/github_username/repo_name.svg?style=for-the-badge
[stars-url]: https://github.com/github_username/repo_name/stargazers
[issues-shield]: https://img.shields.io/github/issues/github_username/repo_name.svg?style=for-the-badge
[issues-url]: https://github.com/github_username/repo_name/issues
[license-shield]: https://img.shields.io/github/license/github_username/repo_name.svg?style=for-the-badge
[license-url]: https://github.com/github_username/repo_name/blob/master/LICENSE.txt
[linkedin-shield]: https://img.shields.io/badge/-LinkedIn-black.svg?style=for-the-badge&logo=linkedin&colorB=555
[linkedin-url]: https://linkedin.com/in/linkedin_username
[product-screenshot]: images/screenshot.png
[Next.js]: https://img.shields.io/badge/next.js-000000?style=for-the-badge&logo=nextdotjs&logoColor=white
[Next-url]: https://nextjs.org/
[React.js]: https://img.shields.io/badge/React-20232A?style=for-the-badge&logo=react&logoColor=61DAFB
[React-url]: https://reactjs.org/
[Vue.js]: https://img.shields.io/badge/Vue.js-35495E?style=for-the-badge&logo=vuedotjs&logoColor=4FC08D
[Vue-url]: https://vuejs.org/
[Angular.io]: https://img.shields.io/badge/Angular-DD0031?style=for-the-badge&logo=angular&logoColor=white
[Angular-url]: https://angular.io/
[Svelte.dev]: https://img.shields.io/badge/Svelte-4A4A55?style=for-the-badge&logo=svelte&logoColor=FF3E00
[Svelte-url]: https://svelte.dev/
[Laravel.com]: https://img.shields.io/badge/Laravel-FF2D20?style=for-the-badge&logo=laravel&logoColor=white
[Laravel-url]: https://laravel.com
[Bootstrap.com]: https://img.shields.io/badge/Bootstrap-563D7C?style=for-the-badge&logo=bootstrap&logoColor=white
[Bootstrap-url]: https://getbootstrap.com
[JQuery.com]: https://img.shields.io/badge/jQuery-0769AD?style=for-the-badge&logo=jquery&logoColor=white
[JQuery-url]: https://jquery.com 
