# Architecture

ShieldCraft is a native Rust desktop application (eframe/egui, OpenGL renderer) with a Rust engine
library. The machine-learning engines run in a separate worker process so that a crash in native
inference code cannot take the application down.

```
┌──────────────────────── ShieldCraft.exe (Rust, egui) ────────────────────────┐
│ Startup screen · menus · toolbar · canvas + selection tools · properties ·   │
│ Effect Monitor · queue / history / log · licenses window · Lantern panel      │
│                    │ background threads, crossbeam channels                   │
│ ┌──────────────────▼──────────────────────────────────────────────────────┐  │
│ │ shieldcraft-core (Rust library)                                          │  │
│ │  image I/O (RGBA8, ICC/EXIF passthrough, safe atomic writes) · engine    │  │
│ │  registry & availability · selection masks · presets · settings ·        │  │
│ │  reports · licenses inventory · Lantern API client                       │  │
│ │  ┌───────────────┐ ┌──────────────┐ ┌─────────────────────────────────┐  │  │
│ │  │ Grid Noise    │ │ Signature    │ │ Hope:RE worker client           │  │  │
│ │  │ Studio (rayon)│ │ Ink          │ │ JSON request file → child proc  │  │  │
│ │  └───────────────┘ └──────────────┘ └───────────────┬─────────────────┘  │  │
│ └─────────────────────────────────────────────────────┼────────────────────┘  │
└───────────────────────────────────────────────────────┼───────────────────────┘
                                                        │ separate process
                              ┌─────────────────────────▼─────────────────────────┐
                              │ shieldcraft-cli.exe hope-worker                    │
                              │ Rust port of Hope:RE's tiled SPSA optimiser (MIT)  │
                              │ ort → onnxruntime.dll 1.24.2 (CPU) → Hope:RE model │
                              └────────────────────────────────────────────────────┘
```

## Why Rust + egui

- One small native executable with a static C runtime; no installer prerequisites.
- The Grid Noise preview renders frames in Rust and uploads them straight to a GPU texture, with no
  per-frame IPC.
- Hope:RE's backend is already Rust + `ort`, so its MIT pipeline ports without a language boundary.

## Module layout

```
crates/shieldcraft-core/          library + shieldcraft-cli (headless CLI and Hope:RE worker)
  src/image_io.rs                 decode/encode PNG/JPEG/WebP/TIFF/BMP/GIF, alpha, ICC & EXIF
  src/engine.rs                   engine ids, method kinds, availability, cancel tokens, errors
  src/engines/gridnoise/          per-frame renderer, still export, GIF/APNG/PNG/FFmpeg export
  src/engines/hope/               SPSA tiler (MIT port), pinned model specs and downloads
  src/engines/signature.rs        Signature Ink (blind_watermark)
  src/engines/worker.rs           runs the Hope:RE worker process, exchanges files, cancels
  src/selection.rs                selection masks: shapes, brush, invert, feather, composite
  src/lantern.rs                  Lantern public-API client (opt-in, user-initiated only)
  src/presets.rs settings.rs report.rs hardware.rs licenses.rs paths.rs
crates/shieldcraft-studio/        the GUI (app, canvas, panels, preview thread, tasks, monitor)
licenses/                         license texts shipped with the app (generated + fixed)
third_party/onnxruntime/          official Microsoft onnxruntime.dll and its notices
packaging/ scripts/               installer, uninstaller, packaging and smoke-test scripts
```

## Engine contract

Each engine declares what kind of protection it is, where it runs, its output geometry and alpha
behaviour, its limitations, and its credit and license, so the UI can disclose all of this before
running. Availability is checked live (worker present, runtime DLL present, model present with the
expected size) and failures are reported as user-readable errors.

## Processing pipeline

1. Import decodes once into an immutable source image. EXIF orientation is applied and recorded.
2. A job is (source, engine, parameter snapshot, optional selection mask, output plan) and runs on
   a background thread. Hope:RE jobs run in the worker process and can be cancelled.
3. With a selection, Grid Noise and Hope:RE results are composited with the original through the
   feathered mask. Hope:RE skips tiles with no selected pixels.
4. Export writes to a temp file, validates it by re-decoding, then renames. The source file is never
   overwritten; name collisions get a numbered suffix.
5. An optional JSON processing report stores file names only, never full paths.

## Privacy

Everything runs locally. The only network access is (a) a Hope:RE model download the user confirms,
from a pinned GitHub release URL verified by SHA-256, and (b) Lantern requests, which are off by
default and only made when the user clicks a Lantern action.
