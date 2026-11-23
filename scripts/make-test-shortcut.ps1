# Makes (or removes) a throwaway Start Menu shortcut to the do-nothing test program, for the
# hand checks of the launcher (docs/backend/01-launcher-apps.md, section 11.1, check 6).
#
#   cargo build -p rain-backend --bin rain-test-app --features test-tools
#   powershell -File scripts/make-test-shortcut.ps1            # creates "Rain Test\Rain Test App.lnk"
#   powershell -File scripts/make-test-shortcut.ps1 -Remove    # deletes it again
#
# The shortcut goes into the current user's Start Menu only. Nothing else is touched.
param([switch]$Remove)

$ErrorActionPreference = 'Stop'
$folder = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Rain Test'
$link = Join-Path $folder 'Rain Test App.lnk'

if ($Remove) {
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    Write-Host 'Removed the Rain Test Start Menu folder.'
    return
}

$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target\debug\rain-test-app.exe'
if (-not (Test-Path $exe)) {
    throw "Build the program first: cargo build -p rain-backend --bin rain-test-app --features test-tools ($exe not found)"
}

New-Item -ItemType Directory -Force $folder | Out-Null
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($link)
$shortcut.TargetPath = $exe
$shortcut.WorkingDirectory = Split-Path -Parent $exe
$shortcut.Save()
Write-Host "Created $link"
