<#
make-ngasak-shortcut.ps1 - one-shot helper: creates a Desktop shortcut that
starts the full ngasak flow (double-click = ngasak run).

The ngasak.exe itself holds the console open at the end of an arg-less
launch, so the plain shortcut is enough.

Usage:
  .\make-ngasak-shortcut.ps1
  .\make-ngasak-shortcut.ps1 -ExePath "D:\tools\ngasak.exe" -Name "NgaSak Run"
#>

param(
    [string] $ExePath = "",
    [string] $Name = "NgaSak"
)

if ("$ExePath" -eq "") {
    $ExePath = Join-Path $PSScriptRoot 'ngasak.exe'
}
if (-not (Test-Path -LiteralPath $ExePath)) {
    # not next to the script: try the Desktop (where the exe usually lands)
    $ExePath = Join-Path ([Environment]::GetFolderPath('Desktop')) 'ngasak.exe'
}
if (-not (Test-Path -LiteralPath $ExePath)) {
    Write-Host "ngasak.exe not found; pass -ExePath" 
    exit 2
}

$desktop = [Environment]::GetFolderPath('Desktop')
$link = Join-Path $desktop "$Name.lnk"

$shell = New-Object -ComObject WScript.Shell
$sc = $shell.CreateShortcut($link)
$sc.TargetPath = $ExePath
$sc.WorkingDirectory = Split-Path $ExePath
$sc.Description = 'NgaSak: ngate connect -> sakura autopush -> scp login-er'
$sc.Save()

Write-Host "Shortcut created: $link"
exit 0
