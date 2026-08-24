# Bundled media

`bundle-media` is a Cargo feature that embeds Plaque Forge's media inside the
binary itself. A bundled binary lists and renders from its internal data plus
the workstation's installed fonts; a normal build reads everything from a
repository checkout.

```bash
cargo build --release --features bundle-media
./target/release/plaque-forge list all
```

Listings behave identically in both builds apart from the source of truth:

| Kind | Non-bundled source | Bundled source |
|---|---|---|
| videos | `assets/*.mp4` | embedded copies |
| styles | `styles/*.toml` | embedded copies |
| plaques | `assets/plaques/catalog.toml` | embedded copy |
| textures | `assets/textures/*.png` | embedded copies |
| fonts | `styles/curated_fonts`, then system fonts | embedded curated fonts, then system fonts |

Curated fonts always lead their listing, one per line in
[`styles/curated_fonts`](../styles/curated_fonts) order, each printed with a
`*` prefix; the remaining installed system families follow alphabetically.

## What is embedded, and what deliberately is not

| Content | Embedded | Notes |
|---|---|---|
| `assets/*.mp4` | yes | source videos for analysis/render/verify |
| `assets/analysis/**` | yes | generated scene caches; render needs them |
| `assets/scenes/**` | yes | human intent consulted by workflows |
| `styles/**` | yes | text-style programs + the curated font list |
| `assets/textures/**` | yes | style-referenced texture images |
| `assets/plaques/**` | yes | standalone plaques with catalog metadata |
| curated fonts | yes | pinned files verbatim; family patterns resolved at build time via `fc-match`, SHA-256 recorded as provenance |
| `assets/homologation/**` | **no** | acceptance evidence stays an on-disk, CI-gated responsibility over validated sources; `homologate` therefore requires a checkout |
| ffmpeg/ffprobe binaries, OpenCV libraries, Python ML worker and models | **impossible** | external runtime dependencies; bundled builds still require them installed |

## Curated font format (`styles/curated_fonts`)

```text
# comment; blank lines ignored
fonts/NotoSerif-Regular.ttf   repository-pinned file (embedded verbatim)
Noto Serif                    system family; bundle embeds fc-match's answer
```

Repository entries must be plain `fonts/<file>` paths. Bundles built on
machines with different fontconfig answers are therefore not byte-identical;
each embedded system font records its digest in the generated manifest so
drift is detectable.

## Running workflows from a bundled binary

The rendering pipeline (OpenCV capture, ffmpeg, texture loading) consumes real
file paths, so bundled builds materialize exactly the embedded files a command
touches into a cache directory mirroring the repository layout
(`${XDG_CACHE_HOME:-~/.cache}/plaque-forge/materialized/<bundle-id>/`,
override with `PLAQUE_FORGE_BUNDLE_CACHE`). Read-side arguments naming
canonical locations (`assets/…`, `styles/…`, `fonts/…`) resolve to internal
data even when a checkout also exists on disk; paths outside those roots pass
through untouched. Write paths (renders, reports, diagnostics) stay wherever
you point them. Derived locations follow the rewritten input, so scene intent,
analysis caches, and any freshly regenerated analysis live inside the same
mirror during a bundled session.

## Build cost

Embedding is off by default and costs nothing in ordinary builds: no new
dependencies, and feature-gated code compiles out entirely.

The payload never passes through rustc. The build script concatenates every
embedded file into one raw blob, converts it into a relocatable object file
with plain binutils (`ld -r -b binary`), and adds that object to the link;
only kilobytes of path/offset tables are compiled. Compiler memory is
therefore independent of media size — the ~400MB bundle builds fine on a
944MB-RAM machine once swap covers the linker's working set (~6 minutes
debug-profile wall time measured). A bundled release build adds LTO time on
top; the resulting debug binary is ~460MB.

Two practical notes: switching link flags rebuilds the dependency graph, so
keep bundled builds in their own target directory
(`CARGO_TARGET_DIR=target/bundle`); and both the blob and its object are
regenerated only when the embedded inputs actually change (content-hash
guard), so repeated `check`/`clippy` runs under `--all-features` skip the
multi-hundred-megabyte preparation entirely — those commands compile only
the kilobyte tables and never link the payload.
