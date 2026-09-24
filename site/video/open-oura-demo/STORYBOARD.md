# Open Oura demo — storyboard

**Message:** your Oura ring works with no cloud: pair on your iPhone, keep a copy on
your own hub, and let your AI agent read it.
**Arc:** demonstration. **Venue:** the setup landing page and the README.
**Format:** 1920×1080, 32 s, 30 fps, music + synthesized SFX, no narration (reads muted).

| # | Beat | Start–end (s) | On screen | Motion | Audio |
| --- | --- | --- | --- | --- | --- |
| 1 | Hook | 0.0–3.4 | Ring mark draws, "Open Oura", "Your ring. Your data. No cloud." | DrawSVG arcs, word stagger, slow dolly | drone, pop at mark |
| 2 | Pair | 3.4–7.2 | iPhone: Pair your ring (real screen); key chip | phone rises, text slides | tick at chip |
| 3 | Scores | 7.2–11.0 | Two iPhones: Summary + Trends; score pills 96 / 89 / 69 | parallax, pill stagger | arpeggio |
| 4 | Hub | 11.0–15.2 | Terminal types `./deploy/install.sh`, real output lines | typewriter, line reveal | typing clicks, chime at "running" |
| 5 | Connect | 15.2–19.6 | Connect page QR in a browser, iPhone prompt "Connect to your hub?" | zoom on QR, phone slides in, tap pulse | tick at tap |
| 6 | Backup | 19.6–23.8 | Hub Sleep page (hypnogram); counter to 116,890 ring events | pan down, count up | arpeggio up |
| 7 | Agent | 23.8–28.2 | `claude mcp add …`, `get_status_now` chip, real JSON excerpt | type, card rise | tick at chip |
| 8 | End | 28.2–32.0 | Mark + "Ring → iPhone → Your hub → Your agent", repo | fade in, resolve | chord resolves |

## Asset audit

- `assets/ios-*.jpg`: simulator in dark mode, iPhone 17 Pro, iOS 26, app built from
  open_health `main` with the seeded `oura.db` (real ring data, 2 nights).
- `assets/web-*.jpg`: the hub web app (oura-hub `main`, dark), served by a local hub
  that the simulator app pushed to. Host shown as
  `homeserver.tail-demo.ts.net` (a Chrome host mapping to 127.0.0.1); token `demo…`.
- Terminal text: the real `deploy/install.sh` output from a Docker run on this Mac,
  with the host name replaced by the demo name and the token masked.
- JSON: the real `get_status_now` result from the local hub.
