# Generates licenses/ and THIRD_PARTY_NOTICES.md from the crates that are actually compiled into
# the Windows build of ShieldCraft.exe (runtime dependencies only; build tools excluded).
# Usage: powershell -ExecutionPolicy Bypass -File scripts\generate-notices.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$lic = Join-Path $root 'licenses'
New-Item -ItemType Directory -Force $lic, (Join-Path $lic 'crates') | Out-Null
# ShieldCraft's own license and notice, so the in-app Licenses window can show them.
Copy-Item (Join-Path $root 'LICENSE-MIT'), (Join-Path $root 'LICENSE-APACHE'), (Join-Path $root 'NOTICE') $lic -Force

$meta = cargo metadata --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
$pkgs = @{}; foreach ($p in $meta.packages) { $pkgs[$p.id] = $p }
$nodes = @{}; foreach ($n in $meta.resolve.nodes) { $nodes[$n.id] = $n }

# Walk normal (runtime) dependencies from both workspace binaries.
$seen = @{}
$queue = New-Object System.Collections.Queue
foreach ($p in $meta.packages | Where-Object { $_.name -in 'shieldcraft-studio', 'shieldcraft-core' }) { $queue.Enqueue($p.id) }
while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    if ($seen.ContainsKey($id)) { continue }
    $seen[$id] = $true
    foreach ($d in $nodes[$id].deps) {
        $normal = $d.dep_kinds | Where-Object { $null -eq $_.kind }
        if ($normal) { $queue.Enqueue($d.pkg) }
    }
}

$rows = @()
foreach ($id in $seen.Keys) {
    $p = $pkgs[$id]
    if ($p.name -like 'shieldcraft-*') { continue }
    $dir = Split-Path $p.manifest_path -Parent
    $files = Get-ChildItem $dir -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE|COPYRIGHT)' }
    $dest = Join-Path $lic "crates\$($p.name)-$($p.version)"
    if ($files) {
        New-Item -ItemType Directory -Force $dest | Out-Null
        $files | Copy-Item -Destination $dest -Force
    }
    $rows += [pscustomobject]@{
        Name = $p.name; Version = $p.version
        License = $(if ($p.license) { $p.license } elseif ($p.license_file) { "See $($p.license_file)" } else { 'UNSPECIFIED' })
        Repo = $p.repository; Files = ($files | ForEach-Object Name) -join ', '
    }
}
$rows = $rows | Sort-Object Name, Version

$dep = @("ShieldCraft - Rust crates compiled into the Windows build", "Generated $(Get-Date -Format 'yyyy-MM-dd') from cargo metadata (runtime dependencies only).", "Full license files for each crate are in licenses/crates/<name>-<version>/.", "")
foreach ($r in $rows) { $dep += "{0} {1}`n    License: {2}`n    Source: {3}`n    License files: {4}" -f $r.Name, $r.Version, $r.License, $r.Repo, $(if ($r.Files) { $r.Files } else { 'none shipped by the crate - see its repository' }) }
$dep | Set-Content (Join-Path $lic 'DEPENDENCIES.txt') -Encoding utf8

$summary = $rows | Group-Object License | Sort-Object Count -Descending | ForEach-Object { "| $($_.Name) | $($_.Count) |" }
$missing = $rows | Where-Object { -not $_.Files }

# Fonts shipped inside egui's default font crate.
$fontDir = Join-Path (Split-Path ($pkgs.Values | Where-Object { $_.name -eq 'epaint_default_fonts' } | Select-Object -First 1).manifest_path -Parent) 'fonts'
$fonts = @('Fonts embedded in ShieldCraft (via epaint_default_fonts)', '')
foreach ($f in 'UFL.txt', 'Hack-Regular.txt', 'OFL.txt', 'emoji-icon-font-mit-license.txt') {
    $fonts += "==================== $f ===================="
    $fonts += Get-Content (Join-Path $fontDir $f) -Raw
}
$fonts | Set-Content (Join-Path $lic 'FONTS.txt') -Encoding utf8

$notice = @"
# Third-Party Notices — ShieldCraft

ShieldCraft integrates or references third-party projects that remain subject to their
respective licenses. Required attribution and license notices are provided below and in the
``licenses`` folder. ShieldCraft does not claim ownership of third-party software.

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

## Rust crates in the Windows build ($($rows.Count) crates)

| License expression | Crates |
|---|---|
$($summary -join "`n")

The full per-crate list is in ``licenses/DEPENDENCIES.txt``. Each crate's own license files are in
``licenses/crates/``. $($missing.Count) crate(s) ship no license file in their package; their
license is taken from the crate metadata listed in DEPENDENCIES.txt.

MPL-2.0 crates ($((($rows | Where-Object { $_.License -match 'MPL' }) | ForEach-Object { "$($_.Name) $($_.Version)" }) -join ', ')) are used unmodified.
Their source code is available from crates.io and the repositories listed in DEPENDENCIES.txt.

## Unresolved questions

- Grid Noise Animator publishes no license. None of its code is included.
- ArtCraft relicensed on 2026-10-10. It is used only as a design reference.
"@
$notice | Set-Content (Join-Path $root 'THIRD_PARTY_NOTICES.md') -Encoding utf8
Copy-Item (Join-Path $root 'THIRD_PARTY_NOTICES.md') (Join-Path $lic 'THIRD_PARTY_NOTICES.md') -Force
"Notices generated: $($rows.Count) crates, $($missing.Count) without bundled license files."
