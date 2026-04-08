# Build release binary and create Windows installer.
#
# Usage:
#   ./script/bundle-windows.ps1                 # uses version from crates/rgitui/Cargo.toml
#   ./script/bundle-windows.ps1 -Version 0.2.0  # overrides the version
#   $env:RGITUI_VERSION="0.2.0"; ./script/bundle-windows.ps1
param(
    [string]$Version = $env:RGITUI_VERSION
)

$ErrorActionPreference = "Stop"

if (-not $Version) {
    $cargoToml = Get-Content "crates\rgitui\Cargo.toml" -Raw
    if ($cargoToml -match '(?m)^version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
    } else {
        Write-Error "Could not determine version from crates/rgitui/Cargo.toml"
        exit 1
    }
}

Write-Host "Building rgitui $Version release binary..."
cargo build --release --package rgitui
if ($LASTEXITCODE -ne 0) { exit 1 }

# Find Inno Setup compiler
$iscc = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
if (-Not (Test-Path $iscc)) {
    $iscc = "C:\Program Files\Inno Setup 6\ISCC.exe"
}
if (-Not (Test-Path $iscc)) {
    Write-Error "Inno Setup 6 not found. Install from https://jrsoftware.org/isdownload.php"
    exit 1
}

Write-Host "Creating installer for version $Version..."
& $iscc "/DMyAppVersion=$Version" "crates\rgitui\resources\windows\rgitui.iss"
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "Done! Installer created in Output/"
