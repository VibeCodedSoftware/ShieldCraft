# ShieldCraft per-user installer (no administrator rights needed).
# Installs to %LOCALAPPDATA%\Programs\ShieldCraft, adds Start Menu (and optional desktop)
# shortcuts, and registers an uninstaller under Settings > Apps.
param([switch]$DesktopShortcut, [switch]$Quiet)
$ErrorActionPreference = 'Stop'
$src = $PSScriptRoot
$dest = Join-Path $env:LOCALAPPDATA 'Programs\ShieldCraft'
$version = '0.1.0'

Get-Process ShieldCraft -ErrorAction SilentlyContinue | ForEach-Object { throw "Please close ShieldCraft before installing." }
New-Item -ItemType Directory -Force $dest | Out-Null
foreach ($item in 'ShieldCraft.exe', 'shieldcraft-cli.exe', 'onnxruntime.dll','THIRD_PARTY_NOTICES.md', 'README.txt', 'uninstall.ps1', 'licenses', 'docs') {
    $p = Join-Path $src $item
    if (Test-Path $p) { Copy-Item $p $dest -Recurse -Force }
}

$shell = New-Object -ComObject WScript.Shell
$exe = Join-Path $dest 'ShieldCraft.exe'
$startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'ShieldCraft.lnk'
$lnk = $shell.CreateShortcut($startMenu); $lnk.TargetPath = $exe; $lnk.WorkingDirectory = $dest; $lnk.IconLocation = "$exe,0"; $lnk.Description = 'ShieldCraft'; $lnk.Save()
if ($DesktopShortcut) {
    $desk = Join-Path ([Environment]::GetFolderPath('Desktop')) 'ShieldCraft.lnk'
    $lnk = $shell.CreateShortcut($desk); $lnk.TargetPath = $exe; $lnk.WorkingDirectory = $dest; $lnk.IconLocation = "$exe,0"; $lnk.Save()
}

$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ShieldCraft'
New-Item -Force $key | Out-Null
$size = [int]((Get-ChildItem $dest -Recurse -File | Measure-Object Length -Sum).Sum / 1KB)
$uninstall = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$dest\uninstall.ps1`""
foreach ($kv in @{ DisplayName = 'ShieldCraft'; DisplayVersion = $version; Publisher = 'ShieldCraft'; DisplayIcon = "$exe,0"; InstallLocation = $dest; UninstallString = $uninstall }.GetEnumerator()) {
    New-ItemProperty -Path $key -Name $kv.Key -Value $kv.Value -PropertyType String -Force | Out-Null
}
New-ItemProperty -Path $key -Name EstimatedSize -Value $size -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $key -Name NoModify -Value 1 -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $key -Name NoRepair -Value 1 -PropertyType DWord -Force | Out-Null

if (-not $Quiet) { Write-Host "ShieldCraft $version installed to $dest. Start it from the Start Menu." }
