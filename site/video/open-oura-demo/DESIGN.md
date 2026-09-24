# Open Oura demo — design

Source: the hub web UI (`oura-hub/web/src/app.css`, dark scheme) and the iOS app
(Apple Health style). Real screenshots from the simulator and a local hub.

## Palette (from app.css)

| Role | Hex |
| --- | --- |
| Background | `#000000` |
| Card | `#1c1c1e` |
| Text | `#ffffff` |
| Secondary text | `#98989f` (video-only: `rgba(235,235,245,.6)` flattened on black) |
| Accent (links, buttons) | `#0a84ff` |
| Readiness | `#30b0c7` |
| Sleep | `#5856d6` |
| Activity | `#ff9500` |
| Good | `#34c759` |

## Type

- Headlines and body: `-apple-system` (SF Pro Display / Text), weight 600 / 400.
- Code and terminal: `ui-monospace` (SF Mono).

## Components rebuilt

- iPhone frame: black body, 64 px radius, 14 px bezel, the real simulator screenshot inside.
- Browser frame: `#1c1c1e` bar with three dots and the hub address, 18 px radius.
- Terminal: `#0d0d0f` window, SF Mono 26 px, `==>` in accent blue, success in green.

## Do / don't

- Do use the product's own screenshots and the real hub output. Mask the token.
- Do keep one accent per beat (the ring colors only on the hook and the end card).
- Don't invent agent replies or metrics. The JSON on screen is the real tool output.
- Don't use shaders or whooshes: the product is calm and private.
