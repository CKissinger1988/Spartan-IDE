# Screenshot inventory and refresh policy

Documentation screenshots are evidence, not product mockups. Each image below records how it was
made and whether it is automatically refreshed after a visual change.

| Location | Surface | Status | Refresh path |
|---|---|---|---|
| `docs/screenshots/web/01-initial-empty-state.png` | Spartan Web, client-side first-open state | Current | GitHub Actions + Chromium |
| `docs/screenshots/site/home.png` | Spartan public landing page | Current | GitHub Actions + Chromium |
| `docs/screenshots/web/02-04-*.png` | Backend-connected Web feature verification | Historical evidence | Re-capture with a real devserver and fixture |
| `docs/screenshots/desktop/*.png` | Desktop verification | Historical evidence | Re-capture with a real Electron/preload bridge or the documented backend WebSocket harness |

## Automated captures

`.github/workflows/screenshots.yml` builds the current Web app, starts the Web preview and static
site, and captures them with Ubuntu Chromium. It runs when the branding, `web/`, `site/`, or mobile
icon assets change, and commits changed screenshot outputs back to the source branch.

The capture script is `web/scripts/capture-brand-screenshots.mjs`. The workflow waits for both
servers before opening either page, so a rendered image always comes from a live served build rather
than a local HTML approximation.

## Capture scope and honesty

The project does not have a Linux display server, a persistent Electron runtime, or an Android/iOS
emulator available in every development environment. Therefore it does not fabricate desktop or
mobile screenshots merely to match a theme change. The historical desktop and backend-connected
web images remain useful verification artifacts, but should not be presented as current branding.

When refreshing one of those images, run the real surface, retain the original evidence level
(actual Electron bridge or actual devserver), replace the file in place, and update the relevant
caption in `desktop/README.md` or `web/README.md`. Do not edit pixels to simulate a newer UI.
