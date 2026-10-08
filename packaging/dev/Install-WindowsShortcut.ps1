param([string]$Distribution = 'LinuxDrop-Dev')
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$launcher = Join-Path $PSScriptRoot 'Start-LinuxDrop.ps1'
$desktop = [Environment]::GetFolderPath('Desktop')
$link = Join-Path $desktop 'LinuxDrop (Ubuntu).lnk'
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($link)
if (Test-Path -LiteralPath $link) {
    if ($shortcut.Arguments -notlike '*Start-LinuxDrop.ps1*') {
        throw 'A different shortcut already uses this name. It was left unchanged.'
    }
}
$shortcut.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$shortcut.Arguments = '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "{0}" -Mode Desktop -Distribution "{1}"' -f $launcher, $Distribution
$shortcut.WorkingDirectory = $repo
$shortcut.Description = 'Open LinuxDrop in its Ubuntu GNOME desktop'
$shortcut.WindowStyle = 7
$shortcut.IconLocation = (Join-Path $env:SystemRoot 'System32\shell32.dll') + ',18'
$assetDir = Join-Path $env:LOCALAPPDATA 'LinuxDrop'
New-Item -ItemType Directory -Force -Path $assetDir | Out-Null
$iconPath = Join-Path $assetDir 'linuxdrop.ico'
$svgPath = Join-Path $repo 'app/linuxdrop/resources/io.github.marius4lui.LinuxDrop.svg'
$temporaryDir = Join-Path $repo '.dev'
New-Item -ItemType Directory -Force -Path $temporaryDir | Out-Null
$temporaryIcon = Join-Path $temporaryDir 'linuxdrop-shortcut.ico'
$linuxSvg = (& wsl.exe -d $Distribution --exec wslpath -a $svgPath.Replace('\', '/')).Trim()
$linuxIcon = (& wsl.exe -d $Distribution --exec wslpath -a $temporaryIcon.Replace('\', '/')).Trim()
@'
import sys, gi
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import GdkPixbuf
image = GdkPixbuf.Pixbuf.new_from_file_at_scale(sys.argv[1], 128, 128, True)
image.savev(sys.argv[2], 'ico', [], [])
'@ | & wsl.exe -d $Distribution --exec /usr/bin/python3 - $linuxSvg $linuxIcon
if ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $temporaryIcon)) {
    Copy-Item -LiteralPath $temporaryIcon -Destination $iconPath -Force
    $shortcut.IconLocation = $iconPath + ',0'
}
$shortcut.Save()
Write-Output "Created $link"
