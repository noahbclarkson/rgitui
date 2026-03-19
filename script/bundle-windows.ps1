# Build release binary and create Windows installer
$ErrorActionPreference = "Stop"

Write-Host "Building rgitui release binary..."
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

Write-Host "Creating installer..."
& $iscc "crates\rgitui\resources\windows\rgitui.iss"
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "Done! Installer created in Output/"
