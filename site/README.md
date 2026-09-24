# site

`index.html` is the setup guide for a new user: the iPhone app, the ring reset and
pairing, the oura-hub server, and an MCP agent. It is a static page; serve this
folder with GitHub Pages or any web server.

## Demo video

`video/open-oura-demo/` is the HyperFrames project for the 32-second demo on the page.

```bash
cd site/video/open-oura-demo
python3 scripts/make-music.py           # the music bed (numpy, ffmpeg)
bash scripts/make-sfx.sh . 19 1.0 32    # the SFX kit (typing1/typing2 need a rename, see STORYBOARD.md)
npx hyperframes render . --output renders/open-oura-demo.mp4 --quality standard --fps 30
```

The screens in `assets/` are real captures: the iOS app in the simulator and the hub
web UI on a local hub. To show new data, capture new screens and replace the files.
The terminal and JSON text in `index.html` is the real script and tool output.
