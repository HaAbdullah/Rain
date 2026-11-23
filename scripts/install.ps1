# Installs Rain for the current user without an installer program: the three programs go into one
# small folder, and the Start menu gets a shortcut. Nothing needs administrator rights.
#
#   npm run install:local                 build the release programs, then install them
#   npm run install:local -- -SkipBuild   install what is already in target\release
#   npm run install:local -- -Uninstall   remove the folder and the shortcut (your data stays)
#
# The installed programs do not need this repository: after an install the `target` folder can be
# deleted (`cargo clean`). Your data folder and the settings in %APPDATA%\Rain are never touched.
param(
    [switch]$SkipBuild,
    [switch]$Uninstall,
    [string]$Target = (Join-Path $env:LOCALAPPDATA 'Programs\Rain')
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$release = Join-Path $repo 'target\release'
$programs = 'rain.exe', 'rain-backend.exe', 'rain-hook.exe'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Rain.lnk'

function Get-RainProcess {
    Get-Process -Name rain, rain-backend, rain-hook -ErrorAction SilentlyContinue
}

function Stop-Rain {
    # The window first: it would start the backend again. Then the backend, the polite way.
    Get-Process -Name rain -ErrorAction SilentlyContinue | Stop-Process -Force
    foreach ($folder in $Target, $release) {
        $backend = Join-Path $folder 'rain-backend.exe'
        if (Test-Path $backend) {
            & $backend --stop | Out-Null
            break
        }
    }
    # Wait until the programs are no longer in use, so that they can be replaced.
    for ($i = 0; $i -lt 50; $i++) {
        if (-not (Get-RainProcess)) { return }
        Start-Sleep -Milliseconds 200
    }
    Get-Process -Name rain-backend -ErrorAction SilentlyContinue | Stop-Process -Force
    for ($i = 0; $i -lt 25; $i++) {
        if (-not (Get-RainProcess)) { return }
        Start-Sleep -Milliseconds 200
    }
}

# Runs a file operation again for a short time: a program that has just ended, or a hook that runs
# for a moment, can keep its file in use for a few hundred milliseconds.
function Invoke-WithRetry([scriptblock]$Action) {
    for ($try = 1; ; $try++) {
        try { & $Action; return }
        catch {
            if ($try -ge 10) { throw }
            Start-Sleep -Milliseconds 300
        }
    }
}

if ($Uninstall) {
    $run = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
    if ($run -and $run.Rain) {
        Write-Warning 'Rain is set to start with Windows. Switch that off in Rain (Settings) before you uninstall.'
        exit 1
    }
    Stop-Rain
    # The folder first: if it cannot be removed, the shortcut still leads to a Rain that works.
    if (Test-Path $Target) { Invoke-WithRetry { Remove-Item $Target -Recurse -Force } }
    if (Test-Path $shortcut) { Remove-Item $shortcut -Force }
    Write-Host "Rain is removed from $Target. Your data folder is untouched."
    exit 0
}

if (-not $SkipBuild) {
    # A Rain that runs from target\release keeps the files that the build must replace.
    $fromBuild = Get-RainProcess | Where-Object { $_.Path -and $_.Path.StartsWith($release, [StringComparison]::OrdinalIgnoreCase) }
    if ($fromBuild) { Stop-Rain }
    Push-Location $repo
    try {
        # npm.cmd, not npm: Windows PowerShell would run npm.ps1 and may take the `--` for itself.
        & npm.cmd run tauri build -- --no-bundle
        if ($LASTEXITCODE -ne 0) { throw 'The build of the window failed.' }
        & cargo build --release -p rain-backend -p rain-hook
        if ($LASTEXITCODE -ne 0) { throw 'The build of the background service failed.' }
    } finally {
        Pop-Location
    }
}

foreach ($name in $programs) {
    if (-not (Test-Path (Join-Path $release $name))) {
        throw "$name is not in $release. Run the script without -SkipBuild."
    }
}

# The new programs are first copied beside the old ones (Rain can stay open during that), and
# only then put in place, so a copy that fails leaves the installed Rain as it was.
New-Item -ItemType Directory -Force $Target | Out-Null
foreach ($name in $programs) {
    Copy-Item (Join-Path $release $name) (Join-Path $Target "$name.new") -Force
}
Stop-Rain
foreach ($name in $programs) {
    Invoke-WithRetry { Move-Item (Join-Path $Target "$name.new") (Join-Path $Target $name) -Force }
}

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = Join-Path $Target 'rain.exe'
$link.WorkingDirectory = $Target
$link.Description = 'Rain'
$link.Save()

$bytes = (Get-ChildItem $Target -File | Measure-Object Length -Sum).Sum
Write-Host ("Rain is installed in {0} ({1:N1} MB). Start it from the Start menu." -f $Target, ($bytes / 1MB))
