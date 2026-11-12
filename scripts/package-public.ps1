# Creates dist\ShieldCraft-<ver>-public.zip containing everything needed to publish:
#   ShieldCraft\       the source repository (no build output) - push this to GitHub
#   release-assets\    the prebuilt Windows package and its checksum - attach to a GitHub release
#   UPLOAD.txt         what goes where
# Run scripts\package.ps1 first so the release package is current.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$version = (Select-String Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$release = "dist\ShieldCraft-$version-win64.zip"
if (-not (Test-Path $release)) { throw "Missing $release - run scripts\package.ps1 first" }

$stage = Join-Path $root "dist\public-staging"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
$src = Join-Path $stage 'ShieldCraft'
New-Item -ItemType Directory -Force $src, "$stage\release-assets" | Out-Null
# Source tree only: build output and local data never leave this machine.
robocopy $root $src /E /XD (Join-Path $root 'target') (Join-Path $root 'dist') (Join-Path $root 'data') /XF portable.flag /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy failed ($LASTEXITCODE)" }
Copy-Item $release, "$release.sha256" "$stage\release-assets"

@"
ShieldCraft $version - public release bundle

ShieldCraft\
    The complete source repository. Upload its contents to a public GitHub repository
    (it already has .gitignore, licenses, notices, README and docs). Build instructions:
    docs\04-BUILD.md.

release-assets\ShieldCraft-$version-win64.zip (+ .sha256)
    The prebuilt Windows app. Attach both files to a GitHub release rather than committing
    them to the repository.

Before publishing, read ShieldCraft\PUBLISHING.md ("Things only you can confirm").
"@ | Set-Content "$stage\UPLOAD.txt" -Encoding utf8

# Refuse to publish anything that names the local user profile.
$profileName = Split-Path $env:USERPROFILE -Leaf
$hits = Get-ChildItem $stage -Recurse -File -Include *.md, *.txt, *.rs, *.toml, *.ps1, *.cmd, *.json, *.lock |
    Select-String -SimpleMatch -Pattern $env:USERPROFILE, "\Users\$profileName\" -List
if ($hits) { $hits | ForEach-Object { $_.Path }; throw 'Local user path found in the public bundle' }

# crates.io packages carry 1970 timestamps, which ZIP cannot represent.
$now = Get-Date
Get-ChildItem $stage -Recurse -File | ForEach-Object { $_.LastWriteTime = $now }
$zip = Join-Path $root "dist\ShieldCraft-$version-public.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item $stage -Recurse -Force
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"Created $zip ($([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB), SHA-256 $hash"
