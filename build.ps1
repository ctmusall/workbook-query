# Build Windows executables into dist\ (runs the test suite first).
# Requires Python 3.10+ from python.org on PATH. Run from PowerShell:
#   powershell -ExecutionPolicy Bypass -File build.ps1
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not (Test-Path .venv)) {
    python -m venv .venv
}
$py = ".\.venv\Scripts\python.exe"
& $py -m pip install -q --upgrade pip
& $py -m pip install -q -e ".[dev]"
if ($LASTEXITCODE -ne 0) { throw "dependency install failed" }

$env:QT_QPA_PLATFORM = "offscreen"
& $py -m pytest --cov=wbquery -q
if ($LASTEXITCODE -ne 0) { throw "tests failed" }
Remove-Item Env:\QT_QPA_PLATFORM

& $py -m PyInstaller --noconfirm --clean wbquery.spec
if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed" }

Write-Host ""
Write-Host "Built:"
Get-ChildItem dist
