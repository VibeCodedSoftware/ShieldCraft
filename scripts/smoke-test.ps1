# Drives the real GUI in its automated smoke-test mode and saves window screenshots.
# Usage: scripts\smoke-test.ps1 -Exe target\debug\ShieldCraft.exe -Image <file> -OutDir <dir>
param(
    [string]$Exe = "target\debug\ShieldCraft.exe",
    [Parameter(Mandatory)][string]$Image,
    [Parameter(Mandatory)][string]$OutDir
)
New-Item -ItemType Directory -Force $OutDir | Out-Null
$env:SHIELDCRAFT_DATA = Join-Path $OutDir 'data'
$cases = @(
    @{ name = 'welcome';   stage = 'welcome';   view = '' },
    @{ name = 'main';      stage = 'main';      view = 'split' },
    @{ name = 'average';   stage = 'main';      view = 'average' },
    @{ name = 'signature'; stage = 'signature'; view = 'side' },
    @{ name = 'licenses';  stage = 'licenses';  view = '' },
    @{ name = 'hope';      stage = 'hope';      view = 'difference' },
    @{ name = 'style';     stage = 'style';     view = 'difference' },
    @{ name = 'engines';   stage = 'engines';   view = '' },
    @{ name = 'provenance'; stage = 'provenance'; view = '' },
    @{ name = 'selection'; stage = 'selection'; view = 'split' },
    @{ name = 'selection_sim'; stage = 'selection_sim'; view = '' }
)
foreach ($c in $cases) {
    $shot = Join-Path $OutDir "shot_$($c.name).png"
    Remove-Item $shot -ErrorAction SilentlyContinue
    $env:SHIELDCRAFT_SMOKE = $shot
    $env:SHIELDCRAFT_SMOKE_STAGE = $c.stage
    $env:SHIELDCRAFT_SMOKE_VIEW = $c.view
    $env:SHIELDCRAFT_SMOKE_OPEN = $Image
    $p = Start-Process $Exe -PassThru
    if (-not $p.WaitForExit(600000)) { $p.Kill(); "$($c.name): TIMEOUT"; continue }
    "{0}: exit {1}, screenshot {2}" -f $c.name, $p.ExitCode, (Test-Path $shot)
}
foreach ($v in 'SHIELDCRAFT_SMOKE', 'SHIELDCRAFT_SMOKE_STAGE', 'SHIELDCRAFT_SMOKE_VIEW', 'SHIELDCRAFT_SMOKE_OPEN', 'SHIELDCRAFT_DATA') { Remove-Item "env:$v" -ErrorAction SilentlyContinue }
