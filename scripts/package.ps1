# Builds the release, regenerates license notices, and creates dist\ShieldCraft-<ver>-win64
# (portable folder) and a .zip of it.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\package.ps1 [-SkipBuild]
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$version = (Select-String Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
# Release binaries are built in their own target dir with source paths remapped, so file paths
# embedded by panics and build scripts don't reveal the builder's user name or folder layout.
$out = Join-Path $root 'target\publish'
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$flags = @('-C', 'target-feature=+crt-static',
    "--remap-path-prefix=$env:USERPROFILE=~",
    "--remap-path-prefix=$cargoHome=cargo",
    "--remap-path-prefix=$out=target",
    "--remap-path-prefix=$root=shieldcraft")
if (-not $SkipBuild) {
    $env:CARGO_TARGET_DIR = $out
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]0x1f
    # cargo reports progress on stderr; judge success by its exit code, not by stderr output.
    $ErrorActionPreference = 'Continue'
    try {
        cargo build --release --workspace
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    } finally {
        $ErrorActionPreference = 'Stop'
        Remove-Item env:CARGO_TARGET_DIR, env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
    }
}
# Refuse to package binaries that still contain the builder's user profile path.
foreach ($exe in 'ShieldCraft.exe', 'shieldcraft-cli.exe') {
    $text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes((Join-Path $out "release\$exe")))
    if ($text.IndexOf($env:USERPROFILE, [StringComparison]::OrdinalIgnoreCase) -ge 0) { throw "$exe still contains the local user profile path" }
}
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\generate-notices.ps1
$name = "ShieldCraft-$version-win64"
$dist = Join-Path $root "dist\$name"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist, "$dist\docs" | Out-Null
Copy-Item "$out\release\ShieldCraft.exe", "$out\release\shieldcraft-cli.exe", third_party\onnxruntime\onnxruntime.dll, THIRD_PARTY_NOTICES.md, LICENSE-MIT, LICENSE-APACHE, NOTICE $dist
Copy-Item licenses $dist -Recurse
Copy-Item packaging\* $dist
Copy-Item docs\*.md "$dist\docs"
$zip = Join-Path $root "dist\$name.zip"
if (Test-Path $zip) { Remove-Item $zip }
# crates.io packages carry 1970 timestamps, which ZIP cannot represent.
$now = Get-Date
Get-ChildItem $dist -Recurse -File | ForEach-Object { $_.LastWriteTime = $now }
Compress-Archive -Path $dist -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$hash  $name.zip" | Set-Content "$zip.sha256" -Encoding ascii
"Packaged $name ($([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB zip), SHA-256 $hash"
