param(
  [string]$Name = "SenseisUpdater"
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command py -ErrorAction SilentlyContinue)) {
  Write-Error "Python launcher 'py' not found. Install Python 3 and ensure 'py' is on PATH."
}

try { Get-Process -Name $Name -ErrorAction Stop | Stop-Process -Force } catch {}
try { taskkill /IM "$Name.exe" /F | Out-Null } catch {}

py -3 -m pip install --upgrade pip
py -3 -m pip install pyinstaller

if (Test-Path "$Name.spec") { Remove-Item -Force "$Name.spec" }
if (Test-Path "dist\$Name.exe") {
  try { attrib -R "dist\$Name.exe" } catch {}
  try { Remove-Item -Force "dist\$Name.exe" } catch {}
}

Remove-Item -Recurse -Force build, dist -ErrorAction SilentlyContinue | Out-Null
New-Item -ItemType Directory -Force -Path build | Out-Null

$entryCode = @"
from sensei_updater.__main__ import run
if __name__ == "__main__":
    run()
"@
$entryPath = Join-Path $PWD "build\entry.py"
Set-Content -Path $entryPath -Value $entryCode -Encoding UTF8

py -3 -m PyInstaller --noconfirm --clean --onefile --console --name $Name --paths src $entryPath

Write-Host "EXE ready: dist\$Name.exe"