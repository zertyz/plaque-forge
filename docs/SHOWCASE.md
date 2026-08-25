# Text-style showcase

`plaque-forge-showcase` is an interactive stage for previewing typography over
the bundled videos. It plays every analyzed asset in a loop and composites a
live-editable title through the exact pipeline the CLI renders with, so what
you approve here is what `render` produces.

```bash
cargo build --bin showcase
./target/debug/showcase                 # from the repository root
./target/debug/showcase --style gold-shine --fit balanced
```

## Keys

| Input | Action |
|---|---|
| PgUp / PgDn | previous / next video (looping; playback never stops for input) |
| Up / Down | cycle style preset |
| Enter | change the title text (default: "Press ENTER to change this text") |
| `/` | font popup: curated fonts first (`*`, from `styles/curated_fonts`), then all system families. Typing starts a live substring search over the full list; Backspace edits, Delete clears. Selection applies immediately; Enter commits, Esc reverts |
| `e` | open the style composer panel |
| composer | Up/Down pick a row, Left/Right adjust numbers or cycle colors, Enter applies add/remove rows, `w` saves, Esc closes |
| `s` | save the current style under a new name (name prompt) |
| `d` | demo mode: each video plays start-to-finish with a random curated-font x preset-style pair (names overlaid); ESC restores your picks |
| `i` | inspect mode: yellow quad = tracked writing surface per frame, solid green = foreground occluders, declared layer masks outlined |
| Space / Left / Right / Home | pause / seek 5s / restart |
| `,` / `.` | step one frame back / forward (auto-pauses) |
| `f` | FAST/FINE preview tier: FAST additionally uses an approximate warp and lighter filtering so playback tracks the source frame rate; FINE runs the exact render pipeline |
| `?` | toggle the key-reference card (also shown automatically for the first seconds) |
| q | quit |

> The Qt highgui backend reserves a few built-in shortcuts (for example
> Ctrl+P opens its properties dialog). Plaque Forge never binds Ctrl-combos,
> so they only ever trigger the toolkit's own helpers.

## Notes

- The window needs a display session (OpenCV highgui/Qt). For headless checks,
  `--smoke N` composites N frames of the first asset without any UI.
- The composer covers every file-free style parameter; image-texture picking
  is deferred.
- Interaction logic lives in `plaque_forge::showcase` modules and is covered
  by unit tests (`cargo test --lib showcase`).

## Automated UI driving

`--driver <script>` runs the interactive loop under program control, and
`--headless` does it without any window — this is how showcase behavior is
tested on machines without a display:

```bash
cat > /tmp/drive.txt <<'EOF'
wait 400
shot /tmp/shots/font-popup.png
press /
text ser          ; live font search
shot /tmp/shots/search.png
press enter
press enter
text NEW TITLE
shot /tmp/shots/typing.png
press enter
press e           ; composer
shot /tmp/shots/composer.png
press esc
press d           ; demo mode
wait 1000
shot /tmp/shots/demo.png
quit
EOF
cargo build --release --bin showcase
./target/release/showcase --headless --driver /tmp/drive.txt --width 960
```

Commands: `wait <ms>`, `press <key>`, `text <chars>`, `shot <path.png>`,
`quit`. Key names: `enter esc up down left right pgup pgdn home end space
comma period delete backspace` or any single character. The run prints its
average frame rate; `PLAQUE_PROFILE=1` adds per-stage timings (decode,
composite, scale, present).

## Performance model

Playback overlaps ffmpeg decoding with composition on a worker thread, and
composited frames are cached at display resolution (budget via
`--cache-mib`, default 800 MiB): once a video has played through once,
looping review and `,`/`.` stepping are served from the cache instantly.
FAST tier lowers supersampling, caps blur/particle counts, and uses an
approximate warp; FINE renders exactly what the CLI renders. The
authoritative warp used by file rendering is untouched by all of this —
homologation contracts keep its bytes stable. Full GPU acceleration
(OpenCL/wgpu) remains a deliberate follow-up: the bundled system OpenCV is
CPU-only, and the preview cache plus multicore OpenCV ops already track
source frame rates on ordinary hardware.
