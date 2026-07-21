import os
from pathlib import Path
from ..core.process import Process
from ..core.console import Console

class SchedulerService:
    def __init__(self, console: Console):
        self.console = console
        self.proc = Process(debug=console.debug, dry_run=console.dry_run)

    def resolve_executable_and_args(self, extra_args: list[str]):
        exe = os.environ.get("SENSEI_EXE_PATH")
        if not exe:
            exe = Path(os.path.realpath(getattr(__import__("sys"), "executable")))
        args = " ".join(extra_args or [])
        return str(exe), args

    def create(self, schedule: str, st_time: str, task_name: str, extra_args: list[str]):
        exe, args = self.resolve_executable_and_args(extra_args)
        if schedule == "weekly":
            rc = self.proc.run_stream(["schtasks", "/Create", "/SC", "WEEKLY", "/D", "MON", "/TN", task_name, "/TR", f'"{exe}" {args}', "/ST", st_time, "/F"])
            return rc == 0
        if schedule == "monthly":
            rc = self.proc.run_stream(["schtasks", "/Create", "/SC", "MONTHLY", "/D", "1", "/TN", task_name, "/TR", f'"{exe}" {args}', "/ST", st_time, "/F"])
            return rc == 0
        return False

    def delete(self, task_name: str):
        rc = self.proc.run_stream(["schtasks", "/Delete", "/TN", task_name, "/F"])
        return rc == 0

    def exists(self, task_name: str):
        rc, _ = self.proc.run_capture(["schtasks", "/Query", "/TN", task_name])
        return rc == 0