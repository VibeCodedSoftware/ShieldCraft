# ShieldCraft 0.1.0 — Feature Status & Test Record

Status as built and tested on 2026-10-21 (Windows 11 x64, 8-core laptop CPU; all engines run on the CPU).

## Engines

| Engine | Status | Verified with real images |
|---|---|---|
| **Grid Noise Studio** (flagship; technique by stelmosfire13-alt) | ✅ Working | GIF, APNG and PNG-sequence exports on a 2400×1500 JPEG and a 512×512 transparent PNG; still exports; frame-average analysis |
| **Signature Ink** | ✅ Working | Embed + read back "Test Artist 2026" with key 1234 on a transparent PNG; alpha preserved |
| **Hope:RE Noise** (method and model by HopeArtOrg) | ✅ Working (added after 0.1.0 packaging) | 512×512 transparent PNG through the GUI and the CLI. At intensity 10 with 30 iterations: max change 13/255, PSNR 32.6 dB, surrogate loss per tile 0.0093 → −0.0139, about 48 ms per model evaluation on the CPU. Transparency preserved |
| **Hope:RE Style Cloak (Glaze mode)** | ✅ Working | Decoy style Cubist, intensity 10 (±9/255), 5 iterations. 512×512 PNG: loss 0.0017 → −0.0134 in 19 s. 2400×1500 JPEG: 104 tiles, 310 s, loss 0.0029 → −0.0078, max change 10/255, PSNR 30.6 dB. Run through the GUI |
| **Hope:RE Concept Poison (Nightshade mode)** | ✅ Working, needs more iterations | 256 px test image, decoy Dog. At intensity 10, 60 iterations did not improve the surrogate loss (0.0146 → 0.0162, flagged by the app). 150 iterations at intensity 10 improved it (→ 0.0110, max change 4/255). 150 iterations at intensity 30 improved it most (→ 0.0035, max change 11/255, PSNR 41.8 dB). Default is therefore intensity 30 / 150 iterations. Every Hope:RE run is checked, and a run whose loss doesn't improve is flagged as not effective |
| **Live preview** (Hope:RE modes) | ✅ Working | Simulated noise at the selected budget, or a magenta Pattern overlay, recomputed instantly when a slider moves. It switches on automatically while adjusting and off when a real result arrives. Labelled as simulated |
| **Auto-iterate** (all Hope:RE modes) | ✅ Working, on by default | After each tile's iterations the loss is measured. A tile that hasn't improved (by more than 1e-4) keeps optimising from where it stopped (same perturbation, momentum and step schedule), up to a cap (default 4× the iteration setting, max 2000). The report lists tiles extended and tiles still not improved; any not-improved tile triggers a warning. Tested on the previously failing case (Concept Poison, 256 px, 60 iterations, intensity 10): without it, 3 of 4 tiles failed (0.0146 → 0.0162, 92 s); with it, 3 tiles were extended, 0 failed (0.0146 → 0.0099, 149 s, max change 8/255) |
| **Provenance (Lantern)** | ✅ Working (public API, no account) | Off until the user turns it on; every request is user-initiated. Hash lookup sends only SHA-256 (`/api/v1/verify/hash`) and shows provenance and history. "Check ID against this file" proves an ID belongs to the exact file. Image matching (`/verify/image`) uploads only after a confirmation dialog. Opt-out registry check. Evidence log plus an HTML evidence report. Live-tested against api.lantern-us.com: health/features OK; local SHA-256 equals Lantern's fingerprint; unregistered file reported as not registered; a matcher timeout was correctly shown as **inconclusive**. **Not available:** registration through the API and the dataset check (both need a Lantern account or partner credentials), and web discovery (Lantern only matches against its own registry). See `PUBLISHING.md` |
| **Effect Monitor** | ✅ Working | Histogram-style panel at the top of the right dock (Window › Effect Monitor). Shows an amplified 1:1 pattern tile, a change histogram, and max / mean / % changed / PSNR for the current engine. Updates live for Grid Noise frames, the Hope:RE simulation while sliders move, and real results. Verified in the GUI for Grid Noise and Style Cloak |
| **Rename to ShieldCraft** | ✅ | Executable, window, installer, data folder (`%LOCALAPPDATA%\ShieldCraft`, migrated automatically from the old `ArtShieldStudio` folder), docs |
| **Difference view** | ✅ Working | Amplified \|protected − original\| (×1–×50) with measured max change and PSNR. Opens automatically when a Hope:RE run finishes. The 224 px tile seams of Hope:RE's method are visible in this view by design |

The Protection menu and Engines tab list only these five engines. Mist v1, the Nightshade research
release and InkShield are not included in any form: they could not be integrated reliably (GPL-3.0
Python/Stable Diffusion stacks needing a GPU; InkShield publishes no license), so they were removed
rather than shown as placeholders. Hope:RE Concept Poison provides the Nightshade-style technique.

## Grid Noise Studio — what the controls do (all implemented)

| Feature | Details |
|---|---|
| Grid | Square or Voronoi cells, 4–256 cells, Voronoi reshaping every N frames, canvas overlay |
| Colour noise | Hue ±0–90°, lightness and saturation ±0–25 %, warp, blur |
| Temporal balance | Random (white), Antiphase (blue), Exact cycle (N frames sum to the artwork); seed |
| Detail split | Separate colour and line layers with a per-cell line gain |
| Static layers | High-frequency grating overlay, tiled low-contrast text, visible mark (corner, size, opacity) |
| Region | Use the selection tools (below); legacy lasso regions from older projects still load |
| Experimental | Iridescent wave, sparkle, cell rainbow (decorative, from the original's test features) |
| Output | GIF, APNG, PNG sequence built in; MP4/WebM with optional audio via FFmpeg; fps, duration, max side, CRF, alpha |
| Storyboard | Combine 2+ open images into one animation, each with its own duration |
| Stills | Static layers at full resolution, or any single frame |
| Preview | Live animation (background thread), timeline scrubbing, frame stepping, Protected / Original / Split / Side-by-side / Overlay / **Frame average** with measured PSNR |

Improvements over the original tool: full resolution (the original caps at 640 px); transparency preserved (the original forces it opaque); seeded and reproducible (the original is unseeded); measured statistics instead of heuristic meters; runs offline (the original loads libraries from CDNs).

Not carried over: drawing/paint tools, video input, the screensaver motion and the soccer mini-game. The "Deep Protect" PGD pass was also left out because it depends on a TensorFlow.js model.

## Hope:RE Noise — how it is integrated

- **Code:** a Rust port of Hope:RE's tiled SPSA optimiser (MIT, credited). It works on 224 px
  tiles with 32 px overlap at full resolution, and is reproducible from a seed. Unlike Hope:RE, a
  missing model is reported rather than silently replaced with plain noise.
- **Worker:** runs in a separate process (`shieldcraft-cli hope-worker`) using Microsoft's official
  `onnxruntime.dll` 1.24.2 on the CPU. A crash in the native runtime cannot take the app down.
  DirectML is not offered, because Hope:RE reports access violations with these models.
- **Model:** `noise_algorithm.onnx` (335 MB) is downloaded only after confirmation. It is checked
  against a pinned SHA-256 that matches Hope:RE's Git LFS pointer.
- **Strength:** uses Hope:RE's own 1–50 scale. ShieldCraft defaults to 10 (±12/255); Hope:RE's
  default of 30 (±37/255) is available as a preset.
- **Measured effect:** each run reports the model's loss before and after optimisation. This
  measures the effect on Hope:RE's surrogate model only, not on other AI systems.
- **Batch:** supported as a batch mode.

## Application

| Area | Status |
|---|---|
| Startup screen (real initialisation steps, licenses, don't-show-again) | ✅ |
| Menus File / Edit / Image / Select / Protection / View / Window / Help + shortcuts (F1 lists them all) | ✅ |
| **Selection (optional)**: rectangle (M), ellipse (Shift+M), lasso (L), brush (B, `[` `]` size); Shift adds, Alt subtracts; Ctrl+A / Ctrl+D / Ctrl+Shift+I invert / Ctrl+H overlay; feathered edges. Applies to Grid Noise and all Hope:RE engines (tiles outside the selection are skipped, pixels outside are byte-identical); Signature Ink always covers the whole image. Batch always processes whole images. | ✅ (unit-tested; masked CLI run verified 0 changed pixels outside) |
| Canvas: zoom, pan, fit, 100 %, pixel grid ≥1600 %, checkerboard, pixel inspector | ✅ |
| Undo / redo of settings, reset | ✅ |
| Import: PNG, JPEG, WebP, TIFF, BMP, GIF (first frame); drag & drop files and folders | ✅ |
| PSD/KRA/HEIC: refused with an explanation and a safe conversion workflow | ✅ |
| Export: PNG, JPEG, lossless WebP, TIFF, BMP; ICC kept by default, EXIF stripped by default | ✅ |
| Safe writing: temp file → re-decode validation → rename; never overwrites the source; collision naming | ✅ (unit-tested + CLI test) |
| Batch: files/folders, per-item status, failures don't stop the batch, retry failed | ✅ |
| Processing reports (.report.json, file names only, no full paths) | ✅ |
| Presets: built-in + user (save/delete) | ✅ |
| Projects (.shieldcraft) save/open | ✅ |
| Third-Party Licenses & Credits window (searchable, full texts) | ✅ |
| Preferences (theme, UI scale, preview size, FFmpeg path, brush size, feather, overlay colour/opacity) | ✅ |
| Dark and light themes, high-DPI | ✅ (dark tested at 125 % scaling) |
| Crash log (`logs\crash.log`), session log | ✅ |

## Tests run

- 25 unit tests (`cargo test --workspace`): selection shapes/add/subtract/invert, brush and compositing, HSL, blur, polygon masks, PNG round-trip with alpha and ICC, JPEG flattening warning, collision naming, source protection, frame reproducibility, alpha preservation, exact-cycle averaging, region masking, Voronoi/detail rendering, GIF/APNG export, signature round-trip.
- CLI end-to-end tests on real images: GIF, APNG, PNG sequence, still, sign/verify, analyse, refusal to overwrite the original.
- GUI smoke tests (`scripts\smoke-test.ps1`), last run on the **extracted release zip** with a fresh data folder: startup screen, split view, frame average, Signature Ink, licenses window, Hope:RE Noise and Style Cloak, Engines tab, Provenance, selection (Grid Noise) and selection (Hope:RE preview). All 11 exited cleanly and produced screenshots.
- Packaged CLI: Concept Poison with a selection mask (tiles outside skipped, 0 changed pixels outside the mask in a sampled check), Signature Ink sign + verify.
- Release audit (0.1.0 final): `cargo clippy` with no correctness findings (remaining notes are style only); fresh-copy build with `--locked`; the release exes contain no user-profile or project paths (enforced by `scripts\package.ps1`); idle CPU ≈ 0 with an image open; about 145 MB working set; clean shutdown.
- Performance: selection overlay and preview-mask rebuilds on a 6000×4000 image went from ~85 ms to ~2–7 ms per brush frame (area-average downscale). Undo history caps selection-mask memory at 512 MB.
- Installer round-trip: install → launch the installed copy → uninstall → folder, shortcut and registry entry removed.
- Dependencies: the exe imports only Windows system DLLs (static C runtime).

## Not tested

- MP4/WebM export: FFmpeg is not installed on the build machine. The code path is implemented but untested.
- The light theme was not captured in the automated screenshots.
- Machines without OpenGL 2.0 drivers (e.g. some virtual machines).
- Windows 10.
- Very large images (above ~8000 px).
