# Launch video

A 1080p60 launch video for Omacus, built from code so it can be re-rendered for each
release.

```sh
video/build.sh
```

| Step | File | What it does |
|---|---|---|
| Answers | `data.mjs` | Runs every line shown in the video through `omacus -`, so each answer is real |
| Narration | `voice.mjs` | One ElevenLabs clip per scene, with word timings for captions. Needs a key in `~/.config/elevenlabs/api_key`; clips are cached |
| Picture | `composition.html`, `composition.js` | The video as a function of time: `render(t)` draws the frame at `t` |
| Frames | `render.mjs` | Steps through at 60fps in headless Chromium and encodes `build/video.mp4` |
| Sound | `audio.py` | An original music bed and sound effects, timed to the composition's events |
| Mix | `mux.mjs` | Narration over ducked music and effects, -14 LUFS, plus a 9:16 cut |

The script lives in the `SCENES` list in `composition.js`. Scene lengths follow the
narration, so editing a line re-times the cuts. Preview single frames with
`node render.mjs --stills 2,10,30` (written to `build/`).

Needs Node, Python 3, ffmpeg and Chrome or Chromium (set `CHROME` if it isn't found).

Output goes to `out/omacus-launch.mp4` and a 9:16 `out/omacus-launch-vertical.mp4` (or
`omacus-launch-preview.mp4` when there's no narration yet).
