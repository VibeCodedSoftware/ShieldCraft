# Phase 1 — Repository & Licensing Audit

> **Outcome (0.1.0):** the "Role" and "Integration strategy" columns below record the plan at audit
> time. The shipped build integrates Grid Noise Studio, Signature Ink and all three Hope:RE modes
> (Noise, Style Cloak, Concept Poison). Mist v1, the Nightshade research release and InkShield are
> **not included in any form** (no code, no adapter, no menu entry). See [03-STATUS.md](03-STATUS.md).

Audit date: 2026-10-21. Sources were downloaded from each repository's `main` branch archive and
inspected directly. GitHub metadata came from the public REST API. Where something could not be
verified, it is marked **UNVERIFIED** or **ASSUMPTION**.

> This is an engineering audit, not legal advice. License conclusions describe what the published
> files say; they do not replace a review by a qualified person if you ever distribute the app.

## Summary matrix

| # | Repository | Language / stack | License (verified) | Role in ShieldCraft | Bundled? |
|---|---|---|---|---|---|
| 1 | storytold/photocraft | Rust, egui, wgpu | MIT OR Apache-2.0 (files present) + trademark carve-out | **Design reference only** | No code copied |
| 2 | storytold/artcraft | Rust/Tauri + TS; new egui app | MIT OR Apache-2.0 since 2026-10-10 (see conflicts) | **Design reference only** | No code copied |
| 3 | stelmosfire13-alt/Grid-Noise-Animator | Single-file HTML/JS (Canvas, WebCodecs, TF.js) | **NONE FOUND** (no LICENSE, no notice, API license endpoint 404) | **Flagship technique — independent Rust implementation** | **No** (no code copied or bundled) |
| 4 | tiffany-mares/inkshield | Python (PyTorch+TF) ML service, Go/Java/TS web stack | **NONE FOUND** (API license endpoint 404) | Optional, user-supplied external tool — **Licensing review required** | **No** |
| 5 | HopeArtOrg/hope-re | Rust/Tauri + Svelte, ONNX Runtime | MIT (© 2025 HopeArtOrg) | **Integrated** (MIT-attributed port of the SPSA/ONNX pipeline + blind watermark) | Port of MIT code; models downloaded on demand |
| 6 | psyker-team/mist (v1.x, **not** Mist v2) | Python 3.8, PyTorch 1.11, CompVis LDM | GPL-3.0 (+ CompVis SD code) | Optional external worker (separate process) | **No** — user installs from upstream |
| 7 | Shawn-Shan/nightshade-release | Python, PyTorch ≥2, diffusers | GPL-3.0 | Optional external worker (separate process) | **No** — user installs from upstream |

---

## 3. Grid Noise Animator (highest priority)

**Confirmed facts (from source):**

- **Form:** 26 standalone HTML files, no build system. `index.html` (Japanese, newest, includes an
  embedded soccer mini-game) is byte-identical to `draw_test.html`. `en.html` plus 16 other locale
  pages are translated copies. Test pages: `adversarial_test`, `ml_protect_test`, `region_test`,
  `screensaver_test`, `jigsaw` (Voronoi + line-art split), `lineart` (colour/line layer split),
  `soccer` (unrelated game). No README.
- **It is fundamentally an animation tool.** Its protection idea is *temporal*: every frame applies
  fresh **zero-mean** per-cell noise, so a viewer's eye averages frames back to the original, while
  any single frame (screenshot, scraped poster frame) is scrambled. Output is video (MP4 H.264 via
  WebCodecs + `mp4-muxer`, WebM via MediaRecorder, or GIF via a built-in encoder). There is no
  "export protected still" path other than saving a drawing.
- **Core per-frame algorithm (`drawNoise`):** image → HSL buffer; image divided into an N×N grid
  (16/32/64/128). Per cell, per frame: random hue angle θ produces a zero-sum RGB chroma offset
  (`amp·cosθ`, `amp·(−½cosθ − (√3/2)sinθ)`, `amp·(−½cosθ + (√3/2)sinθ)`), plus random lightness and
  saturation offsets and an optional random "warp" (per-cell pixel displacement for sampling).
  Optional post blur. Then an optional fixed "ML protection" overlay is added.
- **Exposed parameters:** grid size; noise type white/blue (blue = checkerboard antiphase per frame);
  hue ±0–90°; lightness ±0–25 %; saturation ±0–25 %; *sync cycle* 1–12 (deterministic cosine phase
  cycling so N frames sum to zero); blur 1–10 px; warp 1–20 px; *Enhanced mode* (Voronoi cells,
  seeds re-generated every 1–120 frames); *Detail mode* (blur-radius split into low-freq colour
  layer + high-freq edge layer, edge brightness 0–100 %); experimental Iridescent wave, Sparkle,
  Cell rainbow; *ML protection overlay* (5 summed sinusoidal gratings per channel, period 2–5 px,
  amplitude 6, seeded from image content, mean-removed); *embedded text* tiled at low contrast
  (default "NO-AI-TRAINING", darkness 2–60/255, light/dark); visible watermark ("No AI Learning");
  region lasso (apply inside/outside); orientation rotate/flip; storyboard of 6 panels with
  per-panel durations; painting tools, layers, colour adjust; screensaver motion; audio track;
  export fps 1–60, duration 1–120 s; *Deep Protect*: PGD (ε 2–32/255, 3–30 steps) against
  TF.js MobileNetV2, applied per frame during MP4 export only.
- **Limitations found:** working resolution is capped at **640 px wide** (`MAX_W = 640`) and snapped
  to a multiple of the grid size; **alpha is discarded** (forced to 255); randomness uses unseeded
  `Math.random()` (not reproducible); the on-screen "ML resistance / feature reduction / ×training
  data" meters are a heuristic variance formula, **not a measurement**; TF.js, MobileNet weights and
  mp4-muxer are fetched from CDNs at runtime (not offline).
- **Dependencies:** browser only; TF.js 4.22, @tensorflow-models/mobilenet 2.1 (weights from Google
  storage, ~14 MB), mp4-muxer 5 (all via jsDelivr). CPU/GPU: whatever the browser provides.
- **License: NONE.** No LICENSE file, no header, no README; GitHub's license API returns 404. Under
  default copyright the author retains all rights; public visibility does not grant permission to
  copy, modify or redistribute the code.

**Integration decision:** Do **not** copy, bundle, or translate GNA's code. ShieldCraft
implements the same *published techniques* (temporal zero-mean grid chroma/lightness/saturation
noise, Voronoi cells, detail split, sync cycle, gratings overlay, tiled text, region masks) as
**original Rust code** written for this project. Ideas and algorithms are not protected by
copyright; specific code expression is. Note for honesty: this is *not* a clean-room process in the
strict legal sense, because the original source was studied for this audit. The ShieldCraft engine is
labelled "Grid Noise Studio — independent implementation of techniques demonstrated by Grid Noise
Animator", credits the original project, and does not claim to be it.
**Recommended follow-up:** ask the author (via a GitHub issue) to add a license. If they choose a
permissive license, the engine's attribution can be upgraded accordingly.

## 4. InkShield

- **Stack:** TanStack/React frontend (Vercel), Go gateway, Java Spring auth service, Python FastAPI
  ML service, Postgres, MongoDB, S3, k3s on AWS. It is a **cloud web service with accounts**.
- **Protection core (`services/ml-service/attack.py`):** untargeted L∞ PGD against torchvision
  ResNet-50 (IMAGENET1K_V2), optional ensemble alternating with Keras MobileNetV2 gradients.
  Defaults ε = 0.02, 4 steps (ensemble) / 8 steps (single). CLI: `python attack.py <img> [--ensemble]`
  (writes `test_protected.png` next to the script).
- **Critical limitation:** the image is `Resize(256) → CenterCrop(224)` before the attack and the
  **output is the 224×224 crop** — the artwork's resolution and framing are not preserved.
- **Deps:** torch, torchvision, tensorflow (~2–3 GB of wheels), weights auto-downloaded by
  torchvision/keras (~100 MB + ~14 MB). Runs locally on CPU (README: 30–90 s per image) or CUDA.
- **License: NONE FOUND.** Model weights: torchvision ResNet-50 weights (BSD-3 code; ImageNet-derived
  weights have their own terms), Keras MobileNetV2 (Apache-2.0 code).
- **Decision:** not bundled, not downloaded by the app. Offered only as an **optional user-supplied
  external tool** (user points ShieldCraft at their own local copy) with status
  *Licensing review required*. It is disabled until the user acknowledges this.

## 5. Hope:RE

- **Stack:** Tauri 2 + Svelte 5 frontend; Rust backend with `ort` 2.0.0-rc.12 (ONNX Runtime;
  CUDA + DirectML providers on Windows), `image`, `ndarray`, `blind_watermark` 0.1.
- **Modes actually implemented:**
  - `noise`, `glaze`, `nightshade` — each is a **Hope-authored ONNX model that outputs a scalar loss**
    (CLIP ViT-B/32 image encoder exported from JAX; notebooks in HopeArtOrg/hope-algorithms, MIT).
    Rust optimises a perturbation with **SPSA** (gradient-free, 4 random ± directions per
    iteration, momentum 0.85, edge-weighted perceptual penalty) per 224×224 tile with 32 px overlap,
    then blends tiles back at **full resolution**. Glaze takes a style index
    (Abstract/Impressionist/Cubist/Sketch/Watercolor); Nightshade takes a target index
    (Dog/Cat/Car/Landscape/Person/Building/Food/Abstract).
  - **These are "inspired by" Glaze/Nightshade, not the University of Chicago tools.** ShieldCraft
    labels them "Hope:RE Style Cloak (Glaze-inspired)" and "Hope:RE Concept Poison
    (Nightshade-inspired)".
  - **Signature Ink** — invisible frequency-domain watermark via the `blind_watermark` crate
    (MIT OR Apache-2.0), with optional numeric seed; extraction needs the byte length.
- **Models:** three ONNX files, **335.2 MB each (~1 GB total)**, published as release assets
  (`v2.5.1`). The repo copies are Git-LFS pointers. Download is not hash-pinned upstream (a SHA-256
  is computed but never compared) — ShieldCraft pins the hashes it verifies itself.
- **Behaviour to avoid copying:** if a model is missing, Hope:RE silently applies non-adversarial
  fallback noise and still reports success. ShieldCraft refuses instead and reports
  *Model download required*.
- **License:** MIT © 2025 HopeArtOrg (code); hope-algorithms MIT; CLIP code/weights MIT (OpenAI's
  model card adds intended-use guidance). **Reuse/modify/redistribute: permitted with notice.**
- **Hardware:** CPU works (slow: thousands of 224² CLIP forward passes per tile); DirectML on any
  DX12 GPU; CUDA needs CUDA/cuDNN runtime DLLs.

## 6. Mist v1 (psyker-team/mist — explicitly not Mist v2)

- **Version check:** repository `psyker-team/mist` main branch; `mist_utils.py` declares
  "Configs for Mist V1.2"; entry script is named `mist_v3.py` (script revision, not Mist v2).
  Mist v2 lives in a different repository and was **not** downloaded.
- **Pipeline:** CompVis Stable Diffusion v1.4 LatentDiffusion with an open-gradient `ddpmAttack.py`;
  targeted masked L∞ PGD (`Masked_PGD.py`, advertorch-style) with three modes: textural (VAE latent
  distance to the bundled `MIST.png` target), semantic (diffusion loss), fused
  (textural − rate·semantic). Params: strength ε 1–32, steps 1–1000, size 256/512/768, block
  number 1/2 (low-VRAM), mode 0/1/2, fusion weight, optional mask, experimental no-resize.
- **Output geometry:** square resize to the chosen size by default; "no-resize" keeps aspect ratio
  rounded to multiples of 8·blocks. **Alpha is dropped.** Output is written under `outputs/`
  relative to the working directory.
- **Deps:** Python 3.8.5, PyTorch 1.11, pytorch-lightning 1.4.2, transformers 4.19.2,
  taming-transformers, OpenAI CLIP, advertorch (git). Checkpoint `sd-v1-4.ckpt` **3.97 GB**
  (CreativeML OpenRAIL-M, use-based restrictions; not gated on Hugging Face as of today).
- **Hardware:** effectively NVIDIA CUDA; CPU "works" in code but is impractically slow; ~6+ GB
  VRAM at 512 px, block mode 2 for less.
- **License:** GPL-3.0 (Mist) + CompVis SD code (CreativeML OpenRAIL-M for weights).
- **Decision:** never bundled. Run as a **separate process** from a user-installed upstream clone,
  driven by an ShieldCraft worker script that is itself licensed GPL-3.0. Communication is via
  command line + JSON files (arm's-length; the ShieldCraft binary does not link GPL code).

## 7. Nightshade Release

- **Distinct from Hope:RE:** this is the University of Chicago research code (Shan et al., IEEE S&P
  2024). It shares no code with Hope:RE's "nightshade" mode.
- **Pipeline:** `opt.py` loads `stabilityai/stable-diffusion-2-1` (fp16) via diffusers, generates a
  target image for "A photo of a {target}", then runs 500 steps of L∞ sign-gradient descent on the
  VAE latent distance (ε default 0.04–0.05). `data_extraction.py` selects candidates from a
  LAION-style corpus with CLIP. I/O uses **pickle files** (unsafe for untrusted data).
- **Critical limitations:** output is a **512×512 centre crop**; `device="cuda"` is hard-coded;
  the authors state the code is for research and direct artists to the official Nightshade app.
  **The hard-coded model id `stabilityai/stable-diffusion-2-1` no longer resolves (HTTP 401
  today)**; a community mirror `sd2-community/stable-diffusion-2-1` (OpenRAIL++) exists but has no
  `fp16` branch.
- **License:** GPL-3.0. Model: OpenRAIL++-M.
- **Decision:** optional external worker (GPL-3.0 worker script, image files instead of pickles,
  model id configurable, mirror documented), never bundled.

## 1–2. PhotoCraft and ArtCraft (design references)

- **PhotoCraft:** pure-Rust Photoshop clean-room reimplementation (egui 0.36 UI, wgpu compositor,
  PSD I/O, 26 crates). LICENSE-MIT + LICENSE-APACHE present; NOTICE: ArtCraft name/logos are
  trademarks not covered; bundles Inter/JetBrains Mono (OFL), Lucide (ISC), libfacedetection
  weights (BSD-3).
- **ArtCraft:** Tauri 2 desktop app with a React frontend, plus a new pure-Rust egui app
  (`artcraft-native`, added 2026-10-10).
  **License conflicts found:** (a) GitHub's detector reports Apache-2.0 only, while README/NOTICE
  state *MIT OR Apache-2.0*; (b) both license files were added in a single commit on
  **2026-10-10** titled "Relicense ArtCraft under MIT OR Apache-2.0" — earlier snapshots were under
  different terms; (c) NOTICE lists bundled material **not** licensed at all (Nintendo sound
  effects, meme images, skyboxes, MMD motion data, Mixamo renders).
- **Decision for both:** used only as **design references** (panel organisation, menu conventions,
  information density, egui as a viable professional desktop toolkit). **No code, branding, logos,
  fonts or assets were copied.** They are listed in the licenses window as "Design reference —
  no code incorporated".

## Integration-strategy summary (planned → outcome)

| Engine | Strategy | Status target |
|---|---|---|
| Grid Noise Studio | Original Rust implementation, in-process, multithreaded | **Shipped** |
| Hope:RE Noise / Style Cloak / Concept Poison | MIT-attributed Rust port of SPSA+tiling, ONNX Runtime in an isolated worker, models on demand | **Shipped** (Available after model download) |
| Hope:RE Signature Ink | `blind_watermark` crate, in-process | **Shipped** |
| Mist v1 | (planned: external Python worker) | **Dropped** — GPL-3.0 Python + Stable Diffusion stack; not included |
| Nightshade Release | (planned: external Python worker) | **Dropped** — not included; Concept Poison covers the technique class |
| InkShield | (planned: user-supplied tool) | **Dropped** — no license published; not included |
