# Implementation Plan

Reference: `pi-dashboard-spec.md`. Requirements PI-01..13, Runtime, Resolved decisions.

## Current status
- Phases 1–7 software implementation (`dashboard-core`, `dashboard-pi`, `reconcile_power`, `dashboard-web`, systemd units, CI/Pages workflows) is complete and verified locally.
- Default Macroquad presentation includes the lit 3D extruded split-flap clock (header + screensaver) and the 3D solar & weather archipelago diorama (daylight strip).
- Local preview runner: `uv run scripts/preview.py` (defaults to native release; `--mode web` for the WebGL playground).
- Phase 0 hardware checks (Macroquad Wayland frame rate, `wlr-randr` from systemd user timer, blanked-panel touch input, on-device first-frame update health check) remain pending Pi 5 + Touch Display 2 hardware verification.

## Non-goals (carried from spec)
- No Electron/Node/Python runtime on the Pi.
- No on-device config editing UI.
- No cloud rendering.

## Phase 0 — Hardware gate (blocking, do first, on real device)
Depends on: hardware delivery (tomorrow).

1. Flash Raspberry Pi OS Desktop (64-bit) on the Pi 5, attach Touch Display 2 in portrait.
2. Minimal `macroquad` "hello fullscreen" binary: solid background + moving shape + FPS counter text.
   - Exit criteria: sustained frame time acceptable for a clock/countdown UI (target ≥50 FPS; if capped ~30 FPS, confirmed GLFW/Wayland bug — try XWayland (`WAYLAND_DISPLAY` unset, run under Xwayland) or a borderless window sized to 1200×1920 instead of true fullscreen; record which works).
3. Confirm `wlr-randr --output DSI-1 --off` / `--on` actually blanks/restores the panel when run with `WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR` set to the graphical session's values, invoked from a plain shell *and* from a systemd user service (not just an interactive terminal).
   - Exit criteria: documented exact env vars + invocation that works from systemd; note in spec if different from the `DSI-1` assumption (confirm actual output name via `wlr-randr` with no args).
4. If either check fails, stop and replan the Runtime section before Phase 1 — do not build further on an unconfirmed rendering/power stack.

Validation: manual observation on device; record FPS and the working invocation in a short note (append to spec's Runtime section if it changes).

## Phase 1 — Rust workspace skeleton
Depends on: Phase 0 passing (or at least the working macroquad invocation known).

1. `cargo new` workspace with crates:
   - `dashboard-core` (rendering, panel layout, 3D split-flap clock, 3D solar & weather diorama, state machine for normal/standby/override/screensaver) — compiles to both native (ARM64 target for Pi) and `wasm32-unknown-unknown` (for the GitHub Pages playground).
   - `dashboard-pi` (binary): native entry point, wires real data providers, embedded HTTP server (PI-08), systemd-timer-facing override file.
   - `dashboard-web` (binary/wasm target): entry point using fixture data instead of real providers, no HTTP server, no control panel.
2. `board.toml` schema + parser: stations, walk times, coordinates, time zone, competitions, display windows, screensaver rules. Validate on load; reject on startup with a clear error (PI-06 CI will also validate it).
3. Unit tests for `board.toml` parsing (valid + invalid fixtures) — TDD: write failing parser tests first.

Exit criteria: `cargo build` succeeds for both targets; `cargo test` green; `cargo clippy` clean.

## Phase 2 — Core panels (data layer + rendering), one PI requirement at a time
Each sub-step: write the provider/fetch logic with tests against recorded fixture responses first (TDD), then wire rendering.

1. **PI-02** Clock + weather header (simplest, no external auth, low risk) — Open-Meteo client, 15-min refresh, staleness marking.
2. **PI-03** Daylight strip — solar calculation for configured coordinates/date, recompute daily, correct across local-midnight and DST.
3. **PI-01** Departures — SL Transport API client, per-site fetch/filter, walking-time countdown, disruption overlay (120s poll), elapsed-departure removal, stale/unavailable states.
4. **PI-04** Football — two adapters (football-data.org for Serie A, Highlightly for Allsvenskan) behind one trait/interface so panel rendering doesn't care which provider backs a competition; hourly refresh, last-good-table cache per competition/season.

Exit criteria per step: unit tests pass against fixture data; manual render check via `uv run scripts/preview.py` (native release by default; `--mode web` for the WebGL playground) confirms the panel reads correctly in the portrait vertical-stack layout.

## Phase 3 — Schedule, standby, and the state machine
Depends on: Phase 2 (panels exist to show/hide), Phase 0 (`wlr-randr` invocation confirmed).

1. **PI-05** Display-window evaluation in local civil time (weekday/weekend, multiple windows), DST-correct.
2. Systemd user timer: reconciles screen power once a minute + at startup + after deploy, using the confirmed `wlr-randr` invocation. Waking triggers an immediate departures refresh.
3. State machine for `normal` / `scheduled-standby` / `manual-override` / `screensaver`, with standby > screensaver priority already decided.

Exit criteria: on device, a short display window (e.g. 2-minute test window) demonstrably wakes/sleeps the panel within a minute, survives a reboot mid-window, and triggers a departures refresh on wake.

## Phase 4 — Screensaver (PI-07)
Depends on: Phase 3 (state machine exists).

1. Google Calendar client using a service-account credential (provisioned manually on the Pi, outside the repo).
2. Title-match rules (substring/regex) from `board.toml`; union-of-matches activation logic.
3. Poll loop (~1 min), cache last-known event list; on API failure, hold last state and surface a small "calendar unavailable" indicator.
4. Minimal clock-only screensaver view.

Exit criteria: a test calendar event matching a configured rule flips the state machine into `screensaver` and back within a minute of its start/end; simulated API failure (e.g. revoke credential temporarily) leaves state unchanged and shows the indicator.

## Phase 5 — Remote control & status (PI-08)
Depends on: Phase 3 (state machine + override file already exist), Phase 1 (HTTP server scaffolding can start earlier in parallel if useful, but wiring to real state needs Phase 3).

1. Embedded HTTP server (`axum`) in `dashboard-pi`, bearer-token auth middleware reading the token from the systemd `EnvironmentFile`.
2. `POST /control` (`{"action":"on"|"off"|"clear"}`) → atomically writes/clears override-state file; `GET /status` → reads live app state (normal, scheduled-standby, manual-override, screensaver, touch-wake; per-panel freshness, uptime, version).
3. Static client: embedded HTML/JS/CSS (`dashboard-pi/web/control.html`, compiled in via `include_str!`) served same-origin at `/`. One-time token prompt → `localStorage`.

Exit criteria: from a phone/laptop on the same LAN, hitting `POST /control` with `{"action":"off"}` blanks the screen and it stays off until the next schedule boundary (verified against Phase 3's reconciler); `GET /status` reflects reality; wrong/missing token is rejected.

## Phase 6 — Deployment pipeline (PI-06) & touch input (PI-09..13)
Depends on: Phase 1 skeleton existing (per the "hold off on CI until skeleton exists" decision).

1. GitHub Actions: validate `board.toml`, compile workspace on PRs, and on `v*` tags cross-compile ARM64 (`cross`), bundle `dashboard-pi`, `reconcile_power`, `board.toml`, and `deploy/systemd`, generate `.sha256` digest, and publish to GitHub Releases. `cargo test`, `cargo clippy`, and `spec.yaml` verification run locally.
2. Pi-side update agent: polls Releases, verifies digest, stages, atomic symlink swap, local health check (confirms first frame rendered), defers activation during standby until next wake, rollback to last-known-good on health-check failure.
3. Offline resilience: cold boot with no network still shows clock + last valid cached data per panel, marks unavailable feeds; panels recover independently when connectivity returns (no full restart).
4. Touch input (PI-09..13): pure reducer in `dashboard-core::touch` with 48 px hit targets in `dashboard-core::render`; wake from standby/override-off, screensaver dismissal, standings advance/pause, departure row expansion, rate-limited stale panel refetch, and idle reversion.

Exit criteria: a trivial change (e.g. a walk-time tweak in `board.toml`) reaches the Pi through a real release and applies without a full redeploy of unrelated panels; a deliberately broken build is skipped/rolled back automatically.

## Phase 7 — GitHub Pages playground
Depends on: Phase 1 (`dashboard-web` target exists), can run in parallel with Phases 2-6 once Phase 1 is done.

1. Fixture data sets for each panel (departures, weather, daylight, football) covering normal/stale/unavailable states.
2. Own GitHub Actions workflow: build `dashboard-web` to `wasm32`, deploy to GitHub Pages on push to `main`.

Exit criteria: pushing to `main` updates the live Pages URL within the workflow's run time; the playground never requires or references real credentials.

## Open item carried forward
- Verify Highlightly Football API's attribution/commercial-use terms before Phase 2's Allsvenskan adapter goes live (flagged, not yet independently confirmed).
