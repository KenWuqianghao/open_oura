# site

The Open Oura website, live at <https://open-oura.vercel.app>.

| File | Page |
| --- | --- |
| `index.html` | The landing page |
| `setup.html` | The setup guide (`/setup`): the iPhone app, the ring reset and pairing, the hub, and an MCP agent |
| `assets/` | Screens from the app (simulator, dark mode) and the hub, and the Open Graph image |
| `vercel.json` | Clean URLs (`/setup` serves `setup.html`) |

Both pages are static HTML with no build step. GSAP and the Geist fonts load from
jsDelivr and Google Fonts. Deploy with `vercel deploy --prod` from this folder;
`.vercelignore` keeps the video's source files out of the upload.

## Demo video

`video/open-oura-demo/` is the HyperFrames project for the 32-second demo on the page.

```bash
cd site/video/open-oura-demo
python3 scripts/make-music.py                     # audio/music/theme.wav (needs numpy, ffmpeg)
bash scripts/make-sfx.sh . 19 1.0 32 && mv audio/sfx/typing.wav audio/sfx/typing1.wav
bash scripts/make-sfx.sh . 50 1.4 32 && mv audio/sfx/typing.wav audio/sfx/typing2.wav
rm -f audio/sfx/rush.wav audio/sfx/pad.wav        # not used by this video
npx hyperframes render . --output renders/open-oura-demo.mp4 --quality standard --fps 30
```

The audio files are committed, so the last line alone re-renders the video.

The screens in `assets/` are real captures: the iOS app in the simulator and the hub
web UI on a local hub. To show new data, capture new screens and replace the files.
The terminal and JSON text in `index.html` is the real script and tool output.
