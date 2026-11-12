# Removes ShieldCraft. Your settings, presets and logs in %LOCALAPPDATA%\ShieldCraft are
# kept unless you pass -RemoveUserData. Your artwork and exported files are never touched.
param([switch]$RemoveUserData)
$dest = Join-Path $env:LOCALAPPDATA 'Programs\ShieldCraft'
Get-Process ShieldCraft -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) 'ShieldCraft.lnk') -ErrorAction SilentlyContinue
Remove-Item (Join-Path ([Environment]::GetFolderPath('Desktop')) 'ShieldCraft.lnk') -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ShieldCraft' -Recurse -ErrorAction SilentlyContinue
if ($RemoveUserData) { Remove-Item (Join-Path $env:LOCALAPPDATA 'ShieldCraft') -Recurse -Force -ErrorAction SilentlyContinue }
# The script lives inside the install folder, so delete the folder after this process exits.
Start-Process cmd.exe -ArgumentList "/c timeout /t 2 >nul & rmdir /s /q `"$dest`"" -WindowStyle Hidden
Write-Host 'ShieldCraft was uninstalled.'
