//! Third-party inventory shown in the Licenses window and written to THIRD_PARTY_NOTICES.
//! It describes what this build actually contains; research-only projects are labelled as such.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Third-party code or data compiled into / shipped with this build.
    Included,
    /// Technique re-implemented in ShieldCraft's own code; credited, no code copied.
    IndependentImplementation,
    /// Looked at for UI/architecture ideas only.
    DesignReference,
    /// A hosted third-party service reached over its public API on user request; no code included.
    ExternalService,
    /// ShieldCraft's own code.
    ThisProject,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Included => "Included in this build",
            Status::IndependentImplementation => "Independent implementation (credited)",
            Status::DesignReference => "Design reference only",
            Status::ExternalService => "External service (not open source)",
            Status::ThisProject => "This project",
        }
    }
}

pub struct Component {
    pub name: &'static str,
    pub author: &'static str,
    pub url: &'static str,
    pub status: Status,
    pub license: &'static str,
    /// File in the `licenses/` folder with the full text, if one ships.
    pub license_file: Option<&'static str>,
    pub notes: &'static str,
}

pub const COMPONENTS: &[Component] = &[
    Component {
        name: "ShieldCraft",
        author: "ShieldCraft contributors",
        url: "https://github.com/VibeCodedSoftware/ShieldCraft",
        status: Status::ThisProject,
        license: "MIT OR Apache-2.0",
        license_file: Some("LICENSE-MIT"),
        notes: "ShieldCraft's own code is open source under your choice of the MIT License or the Apache License 2.0. \
                \"Glaze\" and \"Nightshade\" are University of Chicago projects; ShieldCraft's modes with those names are \
                Hope:RE's own methods, named for description only. ShieldCraft is not affiliated with or endorsed by \
                any project or service listed here.",
    },
    Component {
        name: "Lantern",
        author: "Lantern (lantern-us.com)",
        url: "https://lantern-us.com/terms",
        status: Status::ExternalService,
        license: "Proprietary hosted service; no open-source license published. Use is governed by Lantern's Terms.",
        license_file: None,
        notes: "ShieldCraft calls Lantern's documented public API (api.lantern-us.com) only when you click a Lantern \
                action: hash lookups send only the file's SHA-256; image checks upload the image after you confirm. \
                No Lantern code, models or data are included in ShieldCraft.",
    },
    Component {
        name: "Grid Noise Animator",
        author: "stelmosfire13-alt",
        url: "https://github.com/stelmosfire13-alt/Grid-Noise-Animator",
        status: Status::IndependentImplementation,
        license: "No license published",
        license_file: None,
        notes: "Grid Noise Studio implements the temporal grid-noise techniques this project demonstrates \
                (per-cell zero-mean colour noise, Voronoi cells, detail split, sync cycles, overlays, region \
                masks, storyboards). The code is ShieldCraft's own. No source, text or assets from the original \
                are included. Thank you to its creator.",
    },
    Component {
        name: "Hope:RE",
        author: "HopeArtOrg",
        url: "https://github.com/HopeArtOrg/hope-re",
        status: Status::Included,
        license: "MIT",
        license_file: Some("Hope-RE-MIT.txt"),
        notes: "Hope:RE Noise, Style Cloak (Glaze mode) and Concept Poison (Nightshade mode): ShieldCraft includes a \
                Rust port of Hope:RE's tiled SPSA optimiser and uses Hope:RE's noise_algorithm.onnx, glaze_algorithm.onnx \
                and nightshade_algorithm.onnx models (each downloaded only on request from the Hope:RE v2.5.1 release). \
                The models contain OpenAI CLIP ViT-B/32 weights (MIT). Signature Ink uses Hope:RE's watermark configuration.",
    },
    Component {
        name: "ONNX Runtime 1.24.2",
        author: "Microsoft Corporation",
        url: "https://github.com/microsoft/onnxruntime",
        status: Status::Included,
        license: "MIT",
        license_file: Some("onnxruntime/LICENSE.txt"),
        notes: "Official onnxruntime.dll, loaded only by the isolated worker that runs the Hope:RE engines. Its own \
                third-party notices are in licenses/onnxruntime/ThirdPartyNotices.txt.",
    },
    Component {
        name: "ort (Rust ONNX Runtime bindings)",
        author: "pyke.io and contributors",
        url: "https://github.com/pykeio/ort",
        status: Status::Included,
        license: "MIT OR Apache-2.0",
        license_file: Some("MIT.txt"),
        notes: "Rust bindings used by the Hope:RE worker.",
    },
    Component {
        name: "blind_watermark (Rust crate)",
        author: "naganohara-yoshino",
        url: "https://github.com/naganohara-yoshino/blind-watermark-rust",
        status: Status::Included,
        license: "MIT OR Apache-2.0",
        license_file: Some("MIT.txt"),
        notes: "Frequency-domain watermark used by Signature Ink.",
    },
    Component {
        name: "egui / eframe / epaint",
        author: "Emil Ernerfeldt and contributors",
        url: "https://github.com/emilk/egui",
        status: Status::Included,
        license: "MIT OR Apache-2.0",
        license_file: Some("MIT.txt"),
        notes: "User interface toolkit.",
    },
    Component {
        name: "egui default fonts (Ubuntu Light, Hack, Noto Emoji, emoji-icon-font)",
        author: "Canonical; Source Foundry; Google; Emil Ernerfeldt",
        url: "https://github.com/emilk/egui/tree/master/crates/epaint_default_fonts",
        status: Status::Included,
        license: "Ubuntu Font Licence 1.0; MIT/Bitstream Vera; SIL OFL 1.1; MIT",
        license_file: Some("FONTS.txt"),
        notes: "Interface text. Hack is also used to draw tiled text and visible marks.",
    },
    Component {
        name: "Phosphor Icons (egui-phosphor)",
        author: "Phosphor Icons; Andrew Lu",
        url: "https://github.com/amPerl/egui-phosphor",
        status: Status::Included,
        license: "MIT",
        license_file: Some("MIT.txt"),
        notes: "Toolbar and menu icons.",
    },
    Component {
        name: "image, png, gif and other Rust crates",
        author: "Various",
        url: "https://crates.io",
        status: Status::Included,
        license: "Mostly MIT / Apache-2.0 / BSD / Zlib (see DEPENDENCIES.txt)",
        license_file: Some("DEPENDENCIES.txt"),
        notes: "Full per-crate list generated from the build's dependency graph.",
    },
    Component {
        name: "PhotoCraft",
        author: "ArtCraft Team and contributors",
        url: "https://github.com/storytold/photocraft",
        status: Status::DesignReference,
        license: "MIT OR Apache-2.0 (brand assets excluded)",
        license_file: None,
        notes: "Studied for panel layout and egui desktop conventions. No code, branding, fonts or assets copied.",
    },
    Component {
        name: "ArtCraft",
        author: "ArtCraft Team and contributors",
        url: "https://github.com/storytold/artcraft",
        status: Status::DesignReference,
        license: "MIT OR Apache-2.0 since 2026-10-10 (some bundled assets unlicensed)",
        license_file: None,
        notes: "Studied for workflow organisation only. No code, branding or assets copied.",
    },
];

pub const STARTUP_STATEMENT: &str = "ShieldCraft integrates or references third-party projects that remain subject to \
their respective licenses. Required attribution and license notices are provided where applicable. Some models, \
assets, and dependencies have separate terms. ShieldCraft does not claim ownership of third-party software. \
View the Third-Party Licenses panel for details.";

/// Read a shipped license text from the `licenses/` folder.
pub fn license_text(file: &str) -> Option<String> {
    let dir = crate::paths::resource_dir("licenses")?;
    std::fs::read_to_string(dir.join(file)).ok()
}
