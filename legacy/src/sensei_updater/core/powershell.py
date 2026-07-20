import os, tempfile
from .process import Process

PS_PREFIX = r'''
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
'''

class PowerShell:
    def __init__(self, proc: Process):
        self.proc = proc
        self.exe = "powershell.exe" if os.name == "nt" else "pwsh"

    def run(self, script: str) -> int:
        full = PS_PREFIX + "\n" + script
        with tempfile.NamedTemporaryFile(delete=False, suffix=".ps1", mode="w", encoding="utf-8") as f:
            f.write(full); path = f.name
        try:
            return self.proc.run_stream([self.exe, "-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File", path])
        finally:
            try: os.remove(path)
            except Exception: pass