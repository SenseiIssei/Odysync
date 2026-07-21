import json
from datetime import datetime
from pathlib import Path

class RunReport:
    """Aggregates results and can export to JSON or TXT."""
    def __init__(self):
        self.started_at = datetime.utcnow().isoformat() + "Z"
        self.finished_at = None
        self.updated = []
        self.interactive = []
        self.reinstalled = []
        self.skipped = []
        self.store_skipped = []
        self.failed = []
        self.driver_success = None
        self.reboot_required = False
        self.notes = []

    def mark_finished(self):
        self.finished_at = datetime.utcnow().isoformat() + "Z"

    def to_json(self) -> str:
        data = {
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "updated": self.updated,
            "interactive": self.interactive,
            "reinstalled": self.reinstalled,
            "skipped": self.skipped,
            "store_skipped": self.store_skipped,
            "failed": self.failed,
            "driver_success": self.driver_success,
            "reboot_required": self.reboot_required,
            "notes": self.notes,
        }
        return json.dumps(data, indent=2)

    def to_txt(self) -> str:
        lines = []
        lines.append("Sensei's Updater — Run Report")
        lines.append(f"Started:  {self.started_at}")
        lines.append(f"Finished: {self.finished_at or '-'}")
        lines.append("")
        lines.append(f"Driver success: {self.driver_success}")
        lines.append(f"Reboot required: {self.reboot_required}")
        lines.append("")
        def section(title, items):
            lines.append(title + ("" if items else " (none)"))
            for x in items:
                lines.append(f"  - {x}")
            lines.append("")
        section("Updated", self.updated)
        section("Updated (interactive)", self.interactive)
        section("Reinstalled", self.reinstalled)
        section("Skipped", self.skipped)
        section("Store skipped (admin context)", self.store_skipped)
        section("Failed", self.failed)
        if self.notes:
            lines.append("Notes")
            for n in self.notes:
                lines.append(f"  - {n}")
            lines.append("")
        return "\n".join(lines)

    def save(self, fmt: str, out_path: Path):
        out_path.parent.mkdir(parents=True, exist_ok=True)
        if fmt.lower() == "json":
            out_path.write_text(self.to_json(), encoding="utf-8")
        else:
            out_path.write_text(self.to_txt(), encoding="utf-8")