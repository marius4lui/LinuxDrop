param(
    [ValidateSet('App', 'Desktop', 'Build')][string]$Mode = 'Desktop',
    [string]$Distribution = 'LinuxDrop-Dev',
    [string]$LinuxUser = 'linuxdrop'
)
$ErrorActionPreference = 'Stop'
if ($Mode -eq 'Desktop' -and -not $PSBoundParameters.ContainsKey('LinuxUser')) { $LinuxUser = 'ubuntu' }
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$distros = (& wsl.exe --list --quiet) -replace "`0", ''
if (-not ($distros | Where-Object { $_.Trim() -eq $Distribution })) {
    throw "The dedicated WSL distribution '$Distribution' is not installed. Existing distributions will not be changed."
}
$linuxPathOutput = & wsl.exe -d $Distribution --exec wslpath -a $repo.Replace('\', '/')
if ($LASTEXITCODE -ne 0 -or -not $linuxPathOutput) { throw 'Cannot resolve repository path inside WSL.' }
$linuxPath = $linuxPathOutput.Trim()
if ($LASTEXITCODE -ne 0 -or -not $linuxPath.StartsWith('/')) { throw 'Cannot resolve repository path inside WSL.' }
switch ($Mode) {
    'Desktop' {
        & wsl.exe -d $Distribution -u $LinuxUser --cd $linuxPath --exec sh packaging/dev/nested-gnome.sh
    }
    'Build' {
        & wsl.exe -d $Distribution --cd $linuxPath --exec env PATH=/root/.cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin CARGO_TARGET_DIR=/opt/linuxdrop-target CARGO_BUILD_JOBS=3 cargo build --workspace
    }
    'App' {
        # Installed native binary opens as the normal desktop user. No root GUI.
        & wsl.exe -d $Distribution -u $LinuxUser --cd $linuxPath --exec sh packaging/dev/open-installed.sh
    }
}
if ($LASTEXITCODE -ne 0) { throw "LinuxDrop exited with code $LASTEXITCODE." }
