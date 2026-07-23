import os
import json
import time
from pathlib import Path
from datetime import datetime

def _expand_env(path_str: str) -> Path:
    try:
        return Path(os.path.expandvars(path_str)).expanduser()
    except Exception:
        return Path(path_str).expanduser()

def default_reports_dir(cfg_defaults: dict | None = None) -> Path:
    d = cfg_defaults or {}
    out = d.get("out") or r"%LOCALAPPDATA%\SenseiUpdater\last-run.json"
    p = _expand_env(out)
    return p.parent if p.suffix else p

def scan_reports(base_dir: Path) -> list[dict]:
    rows = []
    try:
        if base_dir and base_dir.exists() and base_dir.is_dir():
            for p in base_dir.iterdir():
                if not p.is_file():
                    continue
                ext = p.suffix.lower()
                if ext not in (".json", ".txt"):
                    continue
                try:
                    s = p.stat()
                    rows.append({"path": str(p), "size": int(s.st_size), "mtime": float(s.st_mtime)})
                except Exception:
                    pass
        rows.sort(key=lambda r: r["mtime"], reverse=True)
    except Exception:
        return []
    return rows

def pretty_json(raw: str) -> str:
    try:
        return json.dumps(json.loads(raw), indent=2, ensure_ascii=False)
    except Exception:
        return raw

def _fmt_ts(ts: float) -> str:
    try:
        return datetime.fromtimestamp(float(ts)).strftime("%Y-%m-%d %H:%M")
    except Exception:
        return "unknown time"

def summarize_json(raw: str) -> str:
    try:
        d = json.loads(raw)
    except Exception:
        return pretty_json(raw)
    lines = []
    lines.append("=== Sensei Updater Report ===")
    lines.append("")
    st = d.get("started")
    fi = d.get("finished")
    if st:
        lines.append(f"Started:  {_fmt_ts(st)}")
    if fi:
        lines.append(f"Finished: {_fmt_ts(fi)}")
    lines.append(f"Driver success: {bool(d.get('driver_success'))}")
    lines.append(f"Reboot required: {bool(d.get('reboot_required'))}")
    upd = d.get("updated", []) or []
    itx = d.get("interactive", []) or []
    rei = d.get("reinstalled", []) or []
    fail = d.get("failed", []) or []
    skl = d.get("skipped", []) or []
    ssk = d.get("store_skipped", []) or []
    if upd:
        lines.append("")
        lines.append(f"Updated ({len(upd)}):")
        for x in upd:
            lines.append(f"  + {x}")
    if itx:
        lines.append("")
        lines.append(f"Updated (interactive) ({len(itx)}):")
        for x in itx:
            lines.append(f"  + {x}")
    if rei:
        lines.append("")
        lines.append(f"Reinstalled ({len(rei)}):")
        for x in rei:
            lines.append(f"  r {x}")
    if fail:
        lines.append("")
        lines.append(f"Failed ({len(fail)}):")
        for x in fail:
            lines.append(f"  ! {x}")
    if skl:
        lines.append("")
        lines.append(f"Skipped ({len(skl)}):")
        for x in skl:
            lines.append(f"  ~ {x}")
    if ssk:
        lines.append("")
        lines.append(f"Store skipped ({len(ssk)}):")
        for x in ssk:
            lines.append(f"  ~ {x}")
    notes = d.get("notes", []) or []
    if notes:
        lines.append("")
        lines.append("Notes:")
        for n in notes:
            lines.append(f"  - {n}")
    lines.append("")
    lines.append("—— Raw JSON ————————————————————————————————")
    lines.append(pretty_json(raw))
    return "\n".join(lines)

class RunReport:
    def __init__(self):
        self.started = time.time()
        self.finished = None
        self.updated = []
        self.interactive = []
        self.reinstalled = []
        self.skipped = []
        self.store_skipped = []
        self.failed = []
        self.driver_success = False
        self.reboot_required = False
        self.notes = []

    def mark_finished(self):
        if not self.finished:
            self.finished = time.time()

    def to_dict(self):
        return {
            "started": self.started,
            "finished": self.finished,
            "updated": self.updated,
            "interactive": self.interactive,
            "reinstalled": self.reinstalled,
            "skipped": self.skipped,
            "store_skipped": self.store_skipped,
            "failed": self.failed,
            "driver_success": self.driver_success,
            "reboot_required": self.reboot_required,
            "notes": self.notes
        }

    def save(self, fmt: str, path: Path) -> Path:
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        if fmt.lower() == "json":
            tmp = path.with_suffix(path.suffix + ".tmp")
            tmp.write_text(json.dumps(self.to_dict(), indent=2, ensure_ascii=False), encoding="utf-8")
            try:
                if path.exists():
                    path.unlink(missing_ok=True)
            except Exception:
                pass
            tmp.replace(path)
            return path
        lines = []
        lines.append("Sensei's Updater Report")
        lines.append("")
        lines.append(f"Driver success: {self.driver_success}")
        lines.append(f"Reboot required: {self.reboot_required}")
        def w(label, arr):
            if arr:
                lines.append(f"{label}: " + ", ".join(arr))
        w("Updated", self.updated)
        w("Updated (interactive)", self.interactive)
        w("Reinstalled", self.reinstalled)
        w("Skipped", self.skipped)
        w("Store skipped", self.store_skipped)
        w("Failed", self.failed)
        if self.notes:
            lines.append("Notes: " + "; ".join(self.notes))
        path.write_text("\n".join(lines), encoding="utf-8")
        return path