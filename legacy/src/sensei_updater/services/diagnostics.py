import json
import shutil
import tempfile
from pathlib import Path
from ..core.console import Console

class DiagnosticsService:
    """
    Creates a local, opt-in diagnostics zip with:
      - environment info (OS, PowerShell version, winget version)
      - winget list/upgrade outputs
      - current config (profiles)
      - current run report (JSON/TXT)
    No data is uploaded anywhere automatically.
    """
    def __init__(self, console: Console, cfg, app, system):
        self.console = console
        self.cfg = cfg
        self.app = app
        self.system = system

    def _write_text(self, path: Path, text: str):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text or "", encoding="utf-8", errors="replace")

    def _capture_cmd(self, filename: str, cmd: list[str], tmpdir: Path):
        rc, out = self.app.proc.run_capture(cmd)
        self._write_text(tmpdir / filename, out or f"(exit code {rc}, no output)")
        return rc

    def create(self, zip_path: Path, report, report_fmt: str = "json"):
        tmpdir = Path(tempfile.mkdtemp(prefix="sensei_diag_"))

        try:
            # 1) Basic environment info
            self.console.info("Collecting environment info for diagnostics…")
            ps_env_script = r'''
$psv = $PSVersionTable.PSVersion.ToString()
$osv = (Get-CimInstance Win32_OperatingSystem).Caption + " " + (Get-CimInstance Win32_OperatingSystem).Version
Write-Host "PowerShell: $psv"
Write-Host "Windows: $osv"
'''
            rc, out = self.app.proc.run_capture([self.app.proc and "powershell.exe" or "powershell.exe",
                                                 "-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass",
                                                 "-Command", ps_env_script])
            self._write_text(tmpdir / "env.txt", out or "")

            self._capture_cmd("winget_version.txt", ["winget","--version"], tmpdir)

            # 2) winget outputs
            self.console.info("Capturing winget state…")
            self._capture_cmd("winget_upgrade.txt", ["winget","upgrade"], tmpdir)
            self._capture_cmd("winget_list.txt", ["winget","list"], tmpdir)

            # 3) config snapshot
            cfg_text = json.dumps(self.cfg.data, indent=2)
            self._write_text(tmpdir / "config.json", cfg_text)

            # 4) include current run report
            if report:
                rep_dir = tmpdir / "report"
                rep_dir.mkdir(parents=True, exist_ok=True)
                if report_fmt.lower() == "txt":
                    (rep_dir / "run.txt").write_text(report.to_txt(), encoding="utf-8")
                else:
                    (rep_dir / "run.json").write_text(report.to_json(), encoding="utf-8")

            # 5) pending reboot flag
            try:
                p = self.system.has_pending_reboot()
                self._write_text(tmpdir / "pending_reboot.txt", "True" if p else "False")
            except Exception:
                self._write_text(tmpdir / "pending_reboot.txt", "Unknown")

            # 6) Make zip
            zip_path.parent.mkdir(parents=True, exist_ok=True)
            if zip_path.suffix.lower() == ".zip":
                base = zip_path.with_suffix("")
                archive = shutil.make_archive(str(base), "zip", root_dir=str(tmpdir))
                Path(archive).replace(zip_path)
            else:
                archive = shutil.make_archive(str(zip_path), "zip", root_dir=str(tmpdir))
                zip_path = Path(archive)

        finally:
            try:
                shutil.rmtree(tmpdir, ignore_errors=True)
            except Exception:
                pass

        return zip_path