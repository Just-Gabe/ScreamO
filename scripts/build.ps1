# Build completo: UI (Expo web) + motor (Rust) -> dist\ScreamO
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Rust
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path $cargoBin) { $env:Path = "$cargoBin;$env:Path" }

# Toolchain GNU precisa do dlltool do MinGW (WinLibs via winget).
$winlibs = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory -Filter 'BrechtSanders.WinLibs*' -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($winlibs) { $env:Path = "$($winlibs.FullName)\mingw64\bin;$env:Path" }

Write-Host '==> UI' -ForegroundColor Cyan
Push-Location "$root\ui"
if (-not (Test-Path node_modules)) { npm ci }
npm run build
Pop-Location

Write-Host '==> Motor' -ForegroundColor Cyan
Push-Location "$root\engine"
cargo build --release
Pop-Location

Write-Host '==> Pacote' -ForegroundColor Cyan
$out = "$root\dist\ScreamO"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "$root\engine\target\release\screamo.exe" $out
Copy-Item -Recurse "$root\ui\dist" "$out\ui"
Write-Host "Pronto: $out\screamo.exe" -ForegroundColor Green
