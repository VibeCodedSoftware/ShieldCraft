# Third-Party Notices — ShieldCraft

ShieldCraft integrates or references third-party projects that remain subject to their
respective licenses. Required attribution and license notices are provided below and in the
`licenses` folder. ShieldCraft does not claim ownership of third-party software.

## Credits for projects used or referenced

| Project | Created by | Status in this build | License |
|---|---|---|---|
| [Grid Noise Animator](https://github.com/stelmosfire13-alt/Grid-Noise-Animator) | **stelmosfire13-alt** | Techniques independently re-implemented in Grid Noise Studio; no code included | None published |
| [Hope:RE](https://github.com/HopeArtOrg/hope-re) | **HopeArtOrg** | Hope:RE Noise, Style Cloak and Concept Poison (ported optimiser + noise/glaze/nightshade models, downloaded on request); Signature Ink configuration | MIT (licenses/Hope-RE-MIT.txt); model contains OpenAI CLIP weights (MIT) |
| [Lantern](https://lantern-us.com) | **Lantern** | External hosted service reached over its public API on user request; no code included. Not open source | Proprietary; governed by [Lantern's Terms](https://lantern-us.com/terms) |
| [ONNX Runtime 1.24.2](https://github.com/microsoft/onnxruntime) | **Microsoft** | Included (onnxruntime.dll, used by the Hope:RE worker) | MIT (licenses/onnxruntime/) |
| [blind_watermark](https://github.com/naganohara-yoshino/blind-watermark-rust) | **naganohara-yoshino** | Included (Signature Ink) | MIT OR Apache-2.0 |
| [egui / eframe](https://github.com/emilk/egui) | **Emil Ernerfeldt and contributors** | Included (UI) | MIT OR Apache-2.0 |
| [Phosphor Icons / egui-phosphor](https://github.com/amPerl/egui-phosphor) | **Phosphor Icons; Andrew Lu** | Included (icons) | MIT |
| egui default fonts | Canonical; Source Foundry; Google; Emil Ernerfeldt | Included | UFL 1.0; MIT/Bitstream Vera; OFL 1.1; MIT (licenses/FONTS.txt) |
| [PhotoCraft](https://github.com/storytold/photocraft) | **ArtCraft Team** | Design reference only; nothing copied | MIT OR Apache-2.0 |
| [ArtCraft](https://github.com/storytold/artcraft) | **ArtCraft Team** | Design reference only; nothing copied | MIT OR Apache-2.0 (since 2026-10-10) |

## Rust crates in the Windows build (347 crates)

| License expression | Crates |
|---|---|
| MIT OR Apache-2.0 | 182 |
| MIT | 70 |
| Unicode-3.0 | 18 |
| Apache-2.0 OR MIT | 10 |
| Apache-2.0 | 8 |
| Unlicense OR MIT | 7 |
| MIT/Apache-2.0 | 7 |
| MIT OR Apache-2.0 OR Zlib | 6 |
| BSD-3-Clause | 5 |
| ISC | 4 |
| BSD-2-Clause | 4 |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 |
| Zlib | 2 |
| Unlicense/MIT | 2 |
| BSD-3-Clause OR Apache-2.0 | 2 |
| MIT OR Zlib OR Apache-2.0 | 2 |
| MPL-2.0 | 2 |
| BSL-1.0 | 2 |
| Zlib OR Apache-2.0 OR MIT | 2 |
| Apache-2.0/MIT | 1 |
| Apache-2.0 OR ISC OR MIT | 1 |
| Apache-2.0 / MIT | 1 |
| CDLA-Permissive-2.0 | 1 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 1 |
| 0BSD OR MIT OR Apache-2.0 | 1 |
| Apache-2.0 AND MIT | 1 |
| CC0-1.0 OR Apache-2.0 | 1 |
| Apache-2.0 AND ISC | 1 |
| (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | 1 |

The full per-crate list is in `licenses/DEPENDENCIES.txt`. Each crate's own license files are in
`licenses/crates/`. 28 crate(s) ship no license file in their package; their
license is taken from the crate metadata listed in DEPENDENCIES.txt.

MPL-2.0 crates (colored 3.1.1, option-ext 0.2.0) are used unmodified.
Their source code is available from crates.io and the repositories listed in DEPENDENCIES.txt.

## Unresolved questions

- Grid Noise Animator publishes no license. None of its code is included.
- ArtCraft relicensed on 2026-10-10. It is used only as a design reference.
