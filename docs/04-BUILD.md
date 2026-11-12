# Building & Packaging ShieldCraft (Windows)

## Requirements

- Rust (stable, MSVC toolchain). `rust-toolchain.toml` selects `stable-x86_64-pc-windows-msvc`.
  Crate versions are pinned to work with Rust 1.90.
- Visual Studio 2022 Build Tools (C++ workload) and a Windows 10/11 SDK. These provide the linker and
  `rc.exe` for embedding the icon.
- Optional: FFmpeg on PATH, to test MP4/WebM export.

## Commands

```bash
cargo test --workspace
```

```bash
cargo run --release -p shieldcraft-studio
```

```bash
powershell -ExecutionPolicy Bypass -File scripts\package.ps1
```

`package.ps1` builds the release, regenerates `licenses/` and `THIRD_PARTY_NOTICES.md` from the
real dependency graph (`scripts\generate-notices.ps1`), and produces:

- `dist\ShieldCraft-<version>-win64\` — portable folder. Runs from anywhere. Add an empty
  `portable.flag` to keep settings inside the folder.
- `dist\ShieldCraft-<version>-win64.zip` plus a `.sha256` file.

The package also contains `Install ShieldCraft.cmd`. It runs a per-user installer that needs no
administrator rights: it installs to `%LOCALAPPDATA%\Programs`, adds Start Menu and desktop
shortcuts, and registers an uninstaller under Settings › Apps.

## Smoke tests

```bash
powershell -ExecutionPolicy Bypass -File scripts\smoke-test.ps1 -Exe target\release\ShieldCraft.exe -Image <some image> -OutDir <folder>
```

This drives the real GUI through its environment-variable test mode and saves window screenshots.
It uses a separate data folder, so your own settings are never touched.

## Icon

The icon comes from `assets\icon-source.png`. `crates\shieldcraft-studio\build.rs` crops the blue tile,
applies rounded transparent corners and sharpens the small sizes. It writes a classic multi-size ICO
(BMP frames from 16 to 128 px, a PNG frame at 256 px) and embeds it in the exe. Replace the PNG and
rebuild to change the icon.

## Layout

```
crates/shieldcraft-core     image I/O, engines (Grid Noise, Hope:RE port, Signature Ink), presets, settings,
                          licenses (+ shieldcraft-cli, which is also the isolated Hope:RE worker)
crates/shieldcraft-studio   the desktop app (egui/eframe, OpenGL)
packaging/                installer, uninstaller, README shipped in the package
scripts/                  package, notices, smoke tests
licenses/                 generated license texts (shipped)
third_party/onnxruntime/  official ONNX Runtime 1.24.2 DLL + license (shipped)
docs/                     audit, architecture, status, build
```

## License

ShieldCraft's own code is dual-licensed under MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
Third-party notices are in `NOTICE`, `THIRD_PARTY_NOTICES.md` and `licenses/`. Before publishing a
release, read `PUBLISHING.md`.
