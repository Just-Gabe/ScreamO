# Build completo: UI (Expo web) embutida no motor (Rust) -> dist\ScreamO.exe
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Rust
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path $cargoBin) { $env:Path = "$cargoBin;$env:Path" }

# Toolchain GNU precisa do dlltool/windres do MinGW (WinLibs via winget).
$winlibs = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory -Filter 'BrechtSanders.WinLibs*' -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($winlibs) { $env:Path = "$($winlibs.FullName)\mingw64\bin;$env:Path" }

Write-Host '==> UI' -ForegroundColor Cyan
Push-Location "$root\ui"
if (-not (Test-Path node_modules)) { npm ci }
npm run build
Pop-Location

Write-Host '==> Motor (com a UI embutida)' -ForegroundColor Cyan
Push-Location "$root\engine"
cargo build --release
Pop-Location

Write-Host '==> Executável' -ForegroundColor Cyan
New-Item -ItemType Directory -Force "$root\dist" | Out-Null
Copy-Item -Force "$root\engine\target\release\screamo.exe" "$root\dist\ScreamO.exe"
Write-Host "Pronto: $root\dist\ScreamO.exe (arquivo único)" -ForegroundColor Green
