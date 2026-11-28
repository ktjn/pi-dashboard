# Pi Home Dashboard — Specification

Portable spec for extraction into its own repository. Self-contained: no links back to the sl-dashboard repo.

Wall-mounted dashboard, read from across a room, answering: when should I leave, what's it like outside, how are the teams doing, and (new) should the screen go quiet because I'm busy.

## Requirements

| ID | Requirement | Example acceptance check |
|---|---|---|
| PI-01 | Keep SL departures visible during screen hours. Group by configured station and mode, filter by direction, show line, destination, departure countdown, walking time and relevant disruptions. | With Duvbo configured for a 10-minute walk, an expected departure 14 minutes away reads "Gå om 4 min". Rotating the football table leaves that row visible. |
| PI-02 | Show a persistent local clock and the weather at the configured location (temperature °C, next hour's precipitation probability). | The clock advances through an hour boundary while the weather endpoint is offline. The weather panel identifies the age of its last reading. |
| PI-03 | Show civil dawn, sunrise, sunset, civil dusk and total daylight duration for the local date. | For a fixed Stockholm date, daylight duration equals sunset minus sunrise. Crossing local midnight updates all four times. |
| PI-04 | Show the current season's Allsvenskan standings and those of a second configurable competition: competition, season, position, team, played, goal difference, points. | Both competitions readable within one rotation. Rows follow the provider's ranking, including tied points. Each table identifies its season during the off-season. |
| PI-05 | Support separate weekday and weekend display hours, including multiple windows per day. Switch the physical display into standby outside those windows and wake it automatically. The Pi stays online. | With a 06:00–09:00 window, the backlight wakes at 06:00 and goes dark at 09:00 within one minute. A reboot at 09:15 restores standby. Waking triggers a departures refresh. |
| PI-06 | Deploy code, assets and validated configuration together from GitHub. Support unattended startup and recovery. | A walking-time-only change reaches the Pi through a release. A release that fails to render its first frame rolls back and is skipped on the next update check. |
| PI-07 | Replace the dashboard with a configurable screensaver while a matching Google Calendar event is active (e.g. "Focus", "Away", "Meeting"), and restore the dashboard when the event ends. | With a calendar event titled "Focus 14:00–15:00" and a configured match on "Focus", the screensaver takes over at 14:00 and the dashboard returns at 15:00, even if the event is edited or cancelled mid-way. Calendar API outage leaves the dashboard showing, not stuck on the screensaver. |
| PI-08 | Allow turning the screen off/on remotely over the LAN, as a temporary override on top of PI-05's schedule, and report current status (which of normal/scheduled-standby/manual-override/screensaver is active, per-panel data freshness, uptime, deployed version). | Sending an authenticated "off" request blanks the screen immediately and it stays off until the next scheduled window boundary, then resumes normal scheduling. A status request at any time reports the active state and per-panel freshness without needing SSH. |
| PI-09 | A touch during scheduled standby or manual-override off wakes the screen for a configured duration (default 2 min), then restores the prior power state. Waking triggers a departures refresh. | With the panel blanked, a tap lights it within 1 s and refreshes departures; with no further input it blanks again after the configured duration. A touch in an active window changes nothing. |
| PI-10 | A touch while the screensaver is active shows the dashboard for a configured duration, then the screensaver resumes while its calendar event is still active. | During a "Focus" event a tap shows the dashboard; after the duration with no input the screensaver returns. The event ending restores the dashboard regardless of touch. |
| PI-11 | Football panel: tap advances or pauses rotation; auto-rotation resumes after the idle timeout. | A tap moves to the next competition and pauses; 45 s idle resumes rotation. |
| PI-12 | Departures: tap a row to expand deviations and the next three departures; collapses after the idle timeout. | Tapping a row shows detail; with no input it collapses within the timeout. Header taps do nothing. |
| PI-13 | Tap a stale panel to force a refetch, rate-limited per provider. | Tapping a stale weather panel triggers one fetch; repeated taps within the rate limit trigger none. |

## Runtime

- Single fullscreen Rust application rendered with Macroquad — no Electron/Node/Python runtime on the device.
- Default 3D presentation:
  - Lit 3D extruded split-flap clock in the header and screensaver.
  - 3D solar & weather archipelago diorama in the daylight strip.
- Target panel: official Raspberry Pi Touch Display 2 (10.1", 1200×1920 DSI, natively portrait) on a Raspberry Pi 5. Layout is a vertical stack: clock and weather header, departures as the large middle block, daylight strip with 3D diorama, football in a rotating panel at the bottom. Screensaver (PI-07) fully replaces this layout while active.
- Fetch data outside the render loop. Keep a timestamped snapshot per provider and per station/league so one failing request only affects its panel. Distinguish a successful-empty result from an unavailable feed. Remove elapsed departures, mark stale values, show an unavailable state when no usable data exists. Compute clock, countdowns and daylight locally, never from a remote clock.
- Screen power control uses `wlr-randr --output DSI-1 --off/--on` (not `vcgencmd display_power`, which is non-functional on Pi 5 + Wayland/labwc). Any process invoking it (the systemd timer, or indirectly PI-08) must run with the correct `WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR` for the active graphical session, or the call fails silently.
- Hardware verification gate on Pi 5 + Touch Display 2: confirm Macroquad sustains ~50 FPS in true fullscreen under Wayland with the 3D scenes enabled. Known risk: GLFW/Wayland fullscreen can cap at ~30 FPS, and certain miniquad Wayland-backend configs panic on startup. Fall back to XWayland or a borderless full-resolution window if hit.

## Data sources

| Input | Source and cadence |
|---|---|
| Departures and disruptions | SL Transport API. Start with the web app's 30-second departure and 120-second disruption intervals. Fetch once per distinct site, filter locally. |
| Weather | Open-Meteo candidate. Refresh every 15 minutes. |
| Football | Two separate adapters, one per competition, since no single free provider covers both: Serie A via football-data.org (free tier, 10 req/min); Allsvenskan via Highlightly Football API (free tier, 100 req/day) — verify Highlightly's attribution/commercial-use terms before relying on it. Hourly refresh, cache the last successful table per competition and season. |
| Daylight and clock | Configured coordinates, local system time, IANA time zone (initially Europe/Stockholm). Recompute solar events daily. |
| Calendar (screensaver trigger) | Google Calendar API, one configured calendar, service-account credential (renewable unattended, no browser-based OAuth refresh needed on-device). Poll on an interval short enough to catch event start/end within about a minute; cache the last known event list so a transient API failure doesn't flip screensaver state incorrectly. |

### Reference defaults (from the original React prototype, for continuity — not binding)

```
DEPARTURE_FETCH_INTERVAL_MS = 30_000
DEVIATION_FETCH_INTERVAL_MS = 120_000
CLOCK_TICK_INTERVAL_MS = 1_000
MAJOR_IMPORTANCE_LEVEL = 5

DEFAULT_STATIONS = [
  { siteId: 9324, name: "Duvbo",      mode: "METRO", walkTime: 10, direction: "stockholm" },
  { siteId: 9325, name: "Sundbyberg", mode: "TRAIN", walkTime: 15, direction: "stockholm" },
  { siteId: 9325, name: "Sundbyberg", mode: "TRAM",  walkTime: 13, direction: "stockholm" },
]
```

## Screensaver / Google Calendar integration (PI-07)

- Config declares: calendar ID, OAuth/service-account credential reference, one or more title-match rules (substring or regex), and the screensaver content mode.
- Screensaver content: start with a minimal clock-only view; leave room for a photo-rotation mode later. Do not block on building that now.
- Matching: evaluate the configured calendar's events on each poll; if any event matches a rule and `now` falls within its start/end, activate the screensaver. Multiple matching events behave as a union (any match keeps it active).
- Failure handling: if the calendar API is unreachable or returns an error, keep the current screensaver state unchanged rather than guessing; surface a small unobtrusive "calendar unavailable" indicator so staleness is visible without forcing a jarring mode switch.
- Interaction with PI-05 standby: standby (backlight off) takes priority over the screensaver — don't wake the screen just to show a screensaver, and don't let the screensaver block the standby schedule.
- Credentials: never commit them. Provision the OAuth token or service-account key on the Pi outside the repo (see Configuration below).

## Remote control and status (PI-08)

- A small HTTP server embedded in the main Rust binary (same process as the renderer — no separate service), reachable over the home LAN. Plain HTTP for v1; add TLS/VPN only if ever exposed beyond the LAN.
- Auth: a shared-secret bearer token, provisioned once in the same manually-created systemd `EnvironmentFile` as other credentials.
- `off` request: writes a small override-state file. The existing PI-05 systemd timer (not the HTTP server) remains the sole executor of actual screen power — it reads the override on its next minute-tick. Override holds until the next natural schedule boundary (e.g. the current or next display window edge), then normal scheduling resumes automatically. It is a temporary silence, not an indefinite switch.
- `status` request reports: which of four states is active (`normal` / `scheduled-standby` / `manual-override` / `screensaver`), per-panel last-successful-fetch freshness, app uptime, and deployed version.
- Client: a small static build (e.g. React, compiled ahead of time to plain HTML/JS/CSS — no Node runtime on the Pi) served same-origin by the embedded HTTP server. One-time bearer-token prompt, stored in the browser's `localStorage`.

## GitHub Pages playground

- A browser preview of the dashboard renderer itself (not the PI-08 control client): the same rendering crate compiled to `wasm32` instead of the Pi's native ARM64 target, with the data-fetching layer swapped for static fixtures. No remote-control panel.
- Purpose: iterate on layout, animation and color in a browser without the physical Pi.
- Own GitHub Actions workflow, auto-deploying to GitHub Pages on every push to `main`. Independent of the Pi release pipeline (PI-06) — never touches real credentials, since it only ever runs against fixture data.

## Configuration and delivery

- Version `board.toml` in the repo. It defines: station filters and walking times, coordinates, time zone, competitions, display windows, panel rotation, and the screensaver/calendar rules from PI-07.
- Provision all provider credentials (SL API key if required, Google Calendar OAuth/service-account credentials) separately on the Pi — never in the repo.
- Default display windows: 06:00–09:00 and 16:00–22:30 on weekdays, 08:00–23:00 on weekends.
- Evaluate display windows and calendar event matching in local civil time, including daylight-saving transitions.

### Deployment pipeline

1. **GitHub Actions (CI)**: on PRs and tags, validate `board.toml` and build the workspace (`cargo test`, `cargo clippy`, and `spec.yaml` verification run locally). On `v*` tags, cross-compile ARM64 release binaries (`dashboard-pi`, `reconcile_power`) via `cross` targeting 64-bit Raspberry Pi OS Desktop, bundle them with `board.toml` and `deploy/systemd`, generate a `.sha256` digest, and publish to GitHub Releases. The GitHub Pages playground build is a separate workflow (see above).
2. **Pi-side graphical session**: the installed app runs under a dedicated non-root user inside the graphical session. An independent user-level systemd timer reconciles screen power state at startup, after every deployment, and once a minute thereafter. Confirm Wayland output power control actually works against the physical panel before relying on it.
3. **Pi-side update agent**: periodically polls GitHub Releases, verifies the downloaded bundle's digest, stages it, atomically switches the active version (e.g. symlink swap), and runs a local startup health check. During a scheduled standby window, defer activation until the next wake if the renderer can't confirm it rendered a first frame. Always keep the last known-good bundle available for rollback, and roll back automatically if the new version fails its health check.
4. **Offline resilience**: a cold boot without network access must still show local information (clock, previously cached data where valid) and clearly mark unavailable feeds. When connectivity returns, each panel recovers independently — no full-app restart required.

## Resolved decisions

1. Football: two competitions are Allsvenskan and Serie A, via two separate free-tier adapters (football-data.org for Serie A, Highlightly for Allsvenskan) rather than one paid provider. Verify Highlightly's attribution/commercial-use terms before relying on it in production.
2. Target hardware: Raspberry Pi 5 Starter Kit + official Touch Display 2 (10.1", portrait, 1200×1920 DSI). Layout redesigned as a vertical stack for this aspect ratio (see Runtime).
3. Display windows, coordinates and stale-data thresholds: the spec's example values (06:00–09:00 + 16:00–22:30 weekdays, 08:00–23:00 weekends; Duvbo/Stockholm coordinates; departures 2 min / weather 20 min / football 2 h staleness) are the real v1 defaults, not placeholders.
4. Calendar auth model: service account with calendar sharing (no browser-based OAuth refresh needed on-device).
5. Screensaver visual design: clock-only for v1; photo rotation explicitly deferred.

## Touch input (PI-09 to PI-13)

- Touch is a view-state and bounded-override layer only. It never edits configuration, credentials or permanent power state.
- A pure `InputEvent -> TouchState` reducer lives in `dashboard-core` (testable, wasm-compatible). `dashboard-pi` only translates raw touch events.
- Every touch state reverts to the default view after the idle timeout (default 45 s). Touch targets are at least 48 px. The clock and weather header ignores taps.
- Config: `[touch]` in `board.toml` with `enabled`, `wake_seconds`, `screensaver_dismiss_seconds`, `idle_timeout_seconds`.
- Standby takes priority over touch except for the bounded wake window.
- Hardware gate: confirm touch events reach Macroquad under Wayland, including while the panel is blanked via `wlr-randr`. Risk recorded as `risk-touch-wayland-blanked-input`.
- Status reports add a touch-wake state.

## Open decisions

1. Animation budget: gated on hardware testing. Before any further feature work, confirm on the real device that (a) Macroquad renders acceptably in true fullscreen under Wayland (watch for the known ~30 FPS GLFW/Wayland cap and miniquad Wayland-backend panics — fall back to XWayland or a borderless full-resolution window if hit), and (b) `wlr-randr --output DSI-1 --off/--on` actually toggles the panel from a systemd-timer context with the correct `WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR`. Record frame times once hardware is in hand.

## Non-goals

- No remote/cloud rendering — the Pi renders locally.
- No dashboard editing UI on-device; configuration changes go through the repo and a release.
- No support for runtimes other than the compiled Rust binary (explicitly rules out Electron, a browser kiosk, or a Python/Node service).
