# sensei_updater/services/driver_service.py
from ..core.powershell import PowerShell
from ..core.process import Process
from ..core.admin import require_admin_or_msg
from ..core.console import Console
import json

PS_SCAN = r'''
$ErrorActionPreference="Stop"
try { $null = Get-Module PSWindowsUpdate -ListAvailable } catch {}
try {
  if (-not (Get-Module -ListAvailable -Name PSWindowsUpdate)) {
    if (-not (Get-PackageProvider -Name NuGet -ListAvailable -ErrorAction SilentlyContinue)) { Install-PackageProvider -Name NuGet -Force -Scope CurrentUser | Out-Null }
    if (-not (Get-PSRepository -Name "PSGallery" -ErrorAction SilentlyContinue)) { Register-PSRepository -Name "PSGallery" -SourceLocation "https://www.powershellgallery.com/api/v2" -InstallationPolicy Trusted }
    Install-Module -Name PSWindowsUpdate -Force -Scope CurrentUser -AllowClobber | Out-Null
  }
} catch {}
try { Import-Module PSWindowsUpdate -Force -ErrorAction SilentlyContinue } catch {}
try { Add-WUServiceManager -MicrosoftUpdate -Confirm:$false | Out-Null } catch {}
try {
  $d = Get-WindowsUpdate -MicrosoftUpdate -Category 'Drivers' -ErrorAction Stop
  if ($d) { $d | Select-Object Title,KB,Size,RebootRequired | ConvertTo-Json -Depth 3 } else { "[]"}
} catch {
  if (Get-Command Get-WUList -ErrorAction SilentlyContinue) {
    $d = Get-WUList -MicrosoftUpdate -Category Drivers
    if ($d) { $d | Select-Object Title,KB,Size,RebootRequired | ConvertTo-Json -Depth 3 } else { "[]" }
  } else {
    "[]"
  }
}
'''

PS_INSTALL_PREP = r'''
$ErrorActionPreference="Stop"
try {
  if (-not (Get-PackageProvider -Name NuGet -ListAvailable -ErrorAction SilentlyContinue)) { Install-PackageProvider -Name NuGet -Force -Scope AllUsers | Out-Null }
  try { $repo = Get-PSRepository -Name "PSGallery" -ErrorAction Stop; if ($repo.InstallationPolicy -ne "Trusted") { Set-PSRepository -Name "PSGallery" -InstallationPolicy Trusted } }
  catch { Register-PSRepository -Name "PSGallery" -SourceLocation "https://www.powershellgallery.com/api/v2" -InstallationPolicy Trusted }
  if (-not (Get-Module -ListAvailable -Name PSWindowsUpdate)) { Install-Module -Name PSWindowsUpdate -Force -Scope AllUsers -AllowClobber | Out-Null }
  Import-Module PSWindowsUpdate -Force
  "READY"
} catch { "READY" }
'''

class DriverService:
    def __init__(self, console: Console):
        self.console = console
        self.proc = Process(debug=console.debug, dry_run=console.dry_run)
        self.ps = PowerShell(self.proc)

    def list_available(self, timeout_sec: int = 180):
        rc, out, to = self.proc.run_capture_timeout([self.ps.exe, "-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command", PS_SCAN], timeout_sec)
        if to or rc != 0:
            return []
        try:
            data = json.loads(out) if out else []
        except Exception:
            data = []
        if isinstance(data, dict):
            data = [data]
        rows = []
        for d in data or []:
            rows.append({
                "kb": (d.get("KB") or "").strip(),
                "title": d.get("Title") or "",
                "size": d.get("Size"),
                "reboot": bool(d.get("RebootRequired"))
            })
        return rows

    def _ensure_install_ready(self) -> bool:
        if not require_admin_or_msg(self.console, "Driver Install"):
            return False
        rc, out, to = self.proc.run_capture_timeout([self.ps.exe, "-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command", PS_INSTALL_PREP], 120)
        return not to and rc == 0 and "READY" in (out or "")

    def install_kbs(self, kb_list: list[str]):
        if not kb_list:
            return True, False
        if not self._ensure_install_ready():
            return False, False
        arr = ",".join([f"'{k}'" for k in kb_list if k])
        ps = f'''
$global:RebootRequired = $false
Import-Module PSWindowsUpdate -Force
try {{ Add-WUServiceManager -MicrosoftUpdate -Confirm:$false | Out-Null }} catch {{}}
Install-WindowsUpdate -MicrosoftUpdate -WindowsDriver -KBArticleID @({arr}) -AcceptAll -IgnoreReboot -Verbose
if ($global:RebootRequired) {{ "REBOOT" }} else {{ "OK" }}
'''
        rc, out, to = self.proc.run_capture_timeout([self.ps.exe,"-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command", ps], 1800)
        txt = out or ""
        reboot = "REBOOT" in txt
        ok = (rc == 0) and (not to)
        return ok, reboot

    def update_drivers(self):
        if not self._ensure_install_ready():
            return False, False
        ps = r'''
$global:RebootRequired = $false
Import-Module PSWindowsUpdate -Force
try { Add-WUServiceManager -MicrosoftUpdate -Confirm:$false | Out-Null } catch {}
$drivers = @()
try {
  $drivers = Get-WindowsUpdate -MicrosoftUpdate -Category 'Drivers' -Verbose -ErrorAction Stop
} catch {
  if (Get-Command Get-WUList -ErrorAction SilentlyContinue) {
    $drivers = Get-WUList -MicrosoftUpdate -Category Drivers -Verbose
  }
}
if ($drivers -and $drivers.Count -gt 0) {
  Install-WindowsUpdate -MicrosoftUpdate -Category 'Drivers' -AcceptAll -AutoReboot:$false -Verbose
  if ($global:RebootRequired) { "REBOOT" } else { "OK" }
} else {
  "NONE"
}
'''
        rc, out, to = self.proc.run_capture_timeout([self.ps.exe, "-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command", ps], 2400)
        s = out or ""
        if "NONE" in s:
            return True, False
        if "REBOOT" in s:
            return True, True
        return (rc == 0) and (not to), False