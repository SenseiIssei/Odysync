import os, shutil
from pathlib import Path
from ..core.process import Process
from ..core.powershell import PowerShell
from ..core.admin import require_admin_or_msg

class SystemService:
    def __init__(self, console):
        self.console = console
        self.proc = Process(debug=console.debug, dry_run=console.dry_run)
        self.ps = PowerShell(self.proc)

    # Pending reboot detection (best-effort)
    def has_pending_reboot(self) -> bool:
        ps = r'''
$pending = $false
if (Test-Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired") { $pending = $true }
if (Test-Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending") { $pending = $true }
try {
  $v = (Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager" -ErrorAction Stop).PendingFileRenameOperations
  if ($v) { $pending = $true }
} catch { }
try {
  $s = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing" -ErrorAction Stop)."CBSRebootPending"
  if ($s) { $pending = $true }
} catch { }
if ($pending) { Write-Host "True" } else { Write-Host "False" }
'''
        rc, out = self.proc.run_capture([self.ps.exe, "-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command", ps])
        if rc != 0:
            return False
        return "True" in (out or "")

    def empty_recycle_bin(self):
        if not require_admin_or_msg(self.console, "Empty Recycle Bin"): return
        self.console.header("Emptying Recycle Bin")
        rc = self.ps.run(r'''
try {
  Clear-RecycleBin -Force -ErrorAction SilentlyContinue
  Write-Host "Recycle Bin emptied (or already empty)."
} catch {
  Write-Warning "Failed to empty Recycle Bin: $($_.Exception.Message)"
}
''')
        if rc == 0: self.console.ok("Recycle Bin emptied.")
        else:       self.console.warn("Could not empty Recycle Bin.")

    def cleanup_temp(self):
        if not require_admin_or_msg(self.console, "Clean TEMP folders"): return
        self.console.header("Cleaning TEMP folders")
        candidates = set()
        for var in ("TEMP","TMP"):
            p = os.environ.get(var)
            if p: candidates.add(Path(p))
        candidates.add(Path(r"C:\Windows\Temp"))
        deleted = 0
        for root in candidates:
            try:
                if root.exists():
                    for item in root.iterdir():
                        try:
                            if item.is_file() or item.is_symlink():
                                item.unlink(missing_ok=True); deleted += 1
                            elif item.is_dir():
                                shutil.rmtree(item, ignore_errors=True); deleted += 1
                        except Exception:
                            pass
            except Exception:
                pass
        self.console.ok(f"Removed ~{deleted} temp items (best effort).")

    def dism_sfc(self):
        if not require_admin_or_msg(self.console, "System Health (DISM + SFC)"): return
        self.console.header("System Health: DISM + SFC")
        self.console.info("Running: DISM /Online /Cleanup-Image /ScanHealth")
        self.proc.run_stream(["DISM","/Online","/Cleanup-Image","/ScanHealth"])
        self.console.info("Running: DISM /Online /Cleanup-Image /RestoreHealth")
        self.proc.run_stream(["DISM","/Online","/Cleanup-Image","/RestoreHealth"])
        self.console.info("Running: SFC /scannow")
        self.proc.run_stream(["sfc","/scannow"])
        self.console.ok("Health scan completed. Review output above for any repairs.")

    def show_startup(self):
        self.console.header("Startup Programs")
        self.ps.run(r'''
Get-CimInstance Win32_StartupCommand |
  Select-Object Name, Command, Location |
  Sort-Object Name |
  Format-Table -AutoSize
''')
        
    def open_store_library(self):
        try:
            rc = self.proc.run_stream(["cmd", "/c", "start", "ms-windows-store://downloadsandupdates"])
            return rc == 0
        except Exception:
            return False
