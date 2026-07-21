from ..core.powershell import PowerShell
from ..core.process import Process
from ..core.admin import require_admin_or_msg

class DriverService:
    def __init__(self, console):
        self.console = console
        self.proc = Process(debug=console.debug, dry_run=console.dry_run)
        self.ps = PowerShell(self.proc)

    def _ensure_pswindowsupdate(self) -> bool:
        self.console.header("Preparing PowerShell + PSWindowsUpdate")
        self.console.info("Installing NuGet/PSWindowsUpdate if needed...")
        ps = r'''
if (-not (Get-PackageProvider -Name NuGet -ListAvailable -ErrorAction SilentlyContinue)) {
  Install-PackageProvider -Name NuGet -Force -Scope AllUsers | Out-Null
}
try {
  $repo = Get-PSRepository -Name "PSGallery" -ErrorAction Stop
  if ($repo.InstallationPolicy -ne "Trusted") {
    Set-PSRepository -Name "PSGallery" -InstallationPolicy Trusted
  }
} catch {
  Register-PSRepository -Name "PSGallery" -SourceLocation "https://www.powershellgallery.com/api/v2" -InstallationPolicy Trusted
}
if (-not (Get-Module -ListAvailable -Name PSWindowsUpdate)) {
  Install-Module -Name PSWindowsUpdate -Force -Scope AllUsers -AllowClobber | Out-Null
}
Import-Module PSWindowsUpdate -Force
Write-Host "PSWindowsUpdate is ready."
'''
        rc = self.ps.run(ps)
        if rc != 0:
            self.console.err("Failed to prepare PSWindowsUpdate.")
            return False
        self.console.ok("PSWindowsUpdate is ready.")
        return True

    def create_restore_point(self, desc="Sensei_Restore_Point"):
        if not require_admin_or_msg(self.console, "Create Restore Point"): return False
        self.console.header("Create System Restore Point")
        rc = self.ps.run(f'''
try {{
  Checkpoint-Computer -Description "{desc}" -RestorePointType "MODIFY_SETTINGS"
  Write-Host "Restore point created."
}} catch {{
  Write-Warning "Could not create restore point: $($_.Exception.Message)"
}}
''')
        if rc == 0:
            self.console.ok("Restore point created (or exists today).")
            return True
        else:
            self.console.warn("Could not create restore point (is System Protection enabled?).")
            return False

    def update_drivers(self):
        """
        Returns tuple: (success: bool, reboot_hint: bool)
        """
        if not require_admin_or_msg(self.console, "Driver Updates"): return (False, False)
        self.console.header("Driver Updates via Windows Update (Drivers category)")
        if not self._ensure_pswindowsupdate(): return (False, False)
        self.console.info("Scanning for driver updates (this may take a while)...")
        ps = r'''
Import-Module PSWindowsUpdate -Force
try { Add-WUServiceManager -MicrosoftUpdate -Confirm:$false | Out-Null } catch { }

$reboot = $false
$drivers = $null
try {
  $drivers = Get-WindowsUpdate -MicrosoftUpdate -Category 'Drivers' -Verbose -ErrorAction Stop
} catch {
  if (Get-Command Get-WUList -ErrorAction SilentlyContinue) {
    $drivers = Get-WUList -MicrosoftUpdate -Category Drivers -Verbose
  } else { throw }
}

if ($drivers -and $drivers.Count -gt 0) {
  Write-Host "Driver updates found:"
  $drivers | Select-Object Title, KB, Size | Format-Table -AutoSize
  $ok = $false
  try {
    Install-WindowsUpdate -MicrosoftUpdate -Category 'Drivers' -AcceptAll -AutoReboot:$false -Verbose
    $ok = $true
  } catch {
    try {
      Install-WindowsUpdate -MicrosoftUpdate -Category Drivers -AcceptAll -IgnoreReboot -Verbose
      $ok = $true
    } catch {
      Write-Warning ("Install-WindowsUpdate failed: " + $_.Exception.Message)
    }
  }
  try {
    $global:RebootRequired | Out-Null
    if ($global:RebootRequired) { $reboot = $true }
  } catch { }
  if ($ok) { Write-Host "OK:Drivers"; if ($reboot) { Write-Host "REBOOT:True" } }
  else { Write-Host "OK:Fail" }
} else {
  Write-Host "No driver updates available."
  Write-Host "OK:DriversNone"
}
'''
        rc, out = self.proc.run_capture([self.ps.exe, "-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command", ps])
        print(out or "")
        success = ("OK:Drivers" in (out or "")) or ("OK:DriversNone" in (out or ""))
        reboot_hint = "REBOOT:True" in (out or "")
        if success: self.console.ok("Driver update step finished.")
        else:       self.console.err("Driver update step failed (see output).")
        return (success, reboot_hint)