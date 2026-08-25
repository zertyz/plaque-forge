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
| `f` | FAST/FINE preview tier (FAST caps blur and particle counts so weak machines stay smooth) |
| q | quit |

## Notes

- The window needs a display session (OpenCV highgui/Qt). For headless checks,
  `--smoke N` composites N frames of the first asset without any UI.
- The composer covers every file-free style parameter; image-texture picking
  is deferred.
- Interaction logic lives in `plaque_forge::showcase` modules and is covered
  by unit tests (`cargo test --lib showcase`).
