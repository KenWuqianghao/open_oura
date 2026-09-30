# site

The Open Oura website, live at <https://open-oura.vercel.app>.

| File | Page |
| --- | --- |
| `index.html` | The landing page |
| `setup.html` | The setup guide (`/setup`): the iPhone app, the ring reset and pairing, the hub, and an MCP agent |
| `assets/` | Screens from the app (simulator, dark mode) and the hub, and the Open Graph image |
| `ring3d.js` | The 3D ring (Three.js), shared with the film |
| `vercel.json` | Clean URLs (`/setup` serves `setup.html`) |

Both pages are static HTML with no build step. Three.js, GSAP, and the Geist fonts load
from jsDelivr and Google Fonts. Deploy with `vercel deploy --prod` from this folder;
`.vercelignore` keeps the video's source files out of the upload.

## The 3D ring

`ring3d.js` is a Three.js model of the ring (a turned titanium band with sensor LEDs,
studio reflections, and bloom) plus 2,400 particles that orbit it, fall into the
lanes of one real night's hypnogram, or stream to a target. Everything is a function
of a time `t` and a state object, so the landing page drives it from scroll and the
film drives it frame by frame. The film folder keeps a copy: after a change here, run
`cp ring3d.js video/open-oura-film/ring3d.js`.

## The film

`video/open-oura-film/` is the HyperFrames project for the 49-second film on the page:
the ring, one night, the phone, the features, widgets, live heart rate, the ring
page, the hub, and an agent.

```bash
cd site/video/open-oura-film
python3 scripts/score.py        # audio/score.wav, the music and sound design (numpy)
npx hyperframes render . --output renders/open-oura-film.mp4 --quality high --fps 30
ffmpeg -i renders/open-oura-film.mp4 -c:v libx264 -preset slow -crf 24 -pix_fmt yuv420p \
  -movflags +faststart -c:a aac -b:a 160k renders/open-oura-film-web.mp4
```

The page plays `renders/open-oura-film-web.mp4`; the full-quality master is ignored by
git. The app screens in `assets/` come from the iOS app in the simulator (dark mode)
with the demo database (`oura demo-db`, 45 days of sample data), captured by an
XCUITest driver. The widgets in the film are drawn from the real widget design; the
small one is a crop of the real widget.
