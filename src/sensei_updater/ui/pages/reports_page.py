# sensei_updater/ui/pages/reports_page.py
from __future__ import annotations
import os
import json
from pathlib import Path
from datetime import datetime

from PySide6.QtCore import Qt
from PySide6.QtGui import QGuiApplication
from PySide6.QtWidgets import QWidget, QVBoxLayout, QHBoxLayout, QPushButton, QListWidget, QListWidgetItem, QLabel, QTextEdit

from ..widgets import Header, GlassCard


def _expand_env(path_str: str) -> Path:
    try:
        return Path(os.path.expandvars(path_str)).expanduser()
    except Exception:
        return Path(path_str).expanduser()


class ReportsPage(QWidget):
    def __init__(self, cfg):
        super().__init__()
        self.cfg = cfg
        self._items: list[dict] = []
        self._base_dir = self._resolve_base_dir()
        try:
            self._base_dir.mkdir(parents=True, exist_ok=True)
        except Exception:
            pass

        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        root.addWidget(Header("Reports"))

        card = GlassCard()
        ctrl = QHBoxLayout()
        ctrl.setSpacing(8)
        self.btn_refresh = QPushButton("Refresh"); self.btn_refresh.setObjectName("PrimaryButton")
        self.btn_open_dir = QPushButton("Open Folder"); self.btn_open_dir.setObjectName("PrimaryButton")
        self.btn_open_file = QPushButton("Open File"); self.btn_open_file.setObjectName("PrimaryButton")
        self.btn_delete = QPushButton("Delete"); self.btn_delete.setObjectName("PrimaryButton")
        self.btn_copy = QPushButton("Copy Preview"); self.btn_copy.setObjectName("PrimaryButton")
        ctrl.addWidget(self.btn_refresh)
        ctrl.addWidget(self.btn_open_dir)
        ctrl.addWidget(self.btn_open_file)
        ctrl.addWidget(self.btn_delete)
        ctrl.addWidget(self.btn_copy)
        ctrl.addStretch(1)
        card.v.addLayout(ctrl)

        body = QHBoxLayout()
        body.setSpacing(12)
        self.list = QListWidget()
        self.list.setMinimumWidth(360)
        self.view = QTextEdit()
        self.view.setReadOnly(True)
        body.addWidget(self.list, 0)
        body.addWidget(self.view, 1)
        card.v.addLayout(body, 1)

        pad = QVBoxLayout()
        pad.setContentsMargins(24, 24, 24, 24)
        pad.addWidget(card)
        root.addLayout(pad, 1)

        self.btn_refresh.clicked.connect(self.refresh)
        self.btn_open_dir.clicked.connect(self.open_dir)
        self.btn_open_file.clicked.connect(self.open_file)
        self.btn_delete.clicked.connect(self.delete_selected)
        self.btn_copy.clicked.connect(self.copy_preview)
        self.list.currentItemChanged.connect(self._on_select)
        self.list.itemDoubleClicked.connect(lambda _: self.open_file())

        self.refresh()

    def _resolve_base_dir(self) -> Path:
        d = self.cfg.get_defaults() or {}
        out = d.get("out") or r"%LOCALAPPDATA%\SenseiUpdater\last-run.json"
        out_path = _expand_env(out)
        return out_path.parent if out_path.suffix else out_path

    def _format_dt(self, ts: float) -> str:
        try:
            return datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M")
        except Exception:
            return "unknown time"

    def _pretty_json(self, raw: str) -> str:
        try:
            return json.dumps(json.loads(raw), indent=2, ensure_ascii=False)
        except Exception:
            return raw

    def _summarize_json(self, raw: str) -> str:
        try:
            d = json.loads(raw)
        except Exception:
            return self._pretty_json(raw)
        lines = []
        lines.append("=== Sensei Updater Report ===")
        lines.append("")
        started = d.get("started")
        finished = d.get("finished")
        if started:
            lines.append(f"Started:  {self._format_dt(float(started))}")
        if finished:
            lines.append(f"Finished: {self._format_dt(float(finished))}")
        lines.append(f"Driver success: {bool(d.get('driver_success'))}")
        lines.append(f"Reboot required: {bool(d.get('reboot_required'))}")
        upd = d.get("updated", []) or []
        rei = d.get("reinstalled", []) or []
        fail = d.get("failed", []) or []
        skl = d.get("skipped", []) or []
        ssk = d.get("store_skipped", []) or []
        itx = d.get("interactive", []) or []
        if upd:
            lines.append("")
            lines.append(f"Updated ({len(upd)}):")
            for x in upd: lines.append(f"  + {x}")
        if itx:
            lines.append("")
            lines.append(f"Updated (interactive) ({len(itx)}):")
            for x in itx: lines.append(f"  + {x}")
        if rei:
            lines.append("")
            lines.append(f"Reinstalled ({len(rei)}):")
            for x in rei: lines.append(f"  r {x}")
        if fail:
            lines.append("")
            lines.append(f"Failed ({len(fail)}):")
            for x in fail: lines.append(f"  ! {x}")
        if skl:
            lines.append("")
            lines.append(f"Skipped ({len(skl)}):")
            for x in skl: lines.append(f"  ~ {x}")
        if ssk:
            lines.append("")
            lines.append(f"Store skipped ({len(ssk)}):")
            for x in ssk: lines.append(f"  ~ {x}")
        notes = d.get("notes", []) or []
        if notes:
            lines.append("")
            lines.append("Notes:")
            for n in notes: lines.append(f"  - {n}")
        lines.append("")
        lines.append("—— Raw JSON ————————————————————————————————")
        lines.append(self._pretty_json(raw))
        return "\n".join(lines)

    def _scan_reports(self) -> list[dict]:
        rows: list[dict] = []
        base = self._base_dir
        try:
            base.mkdir(parents=True, exist_ok=True)
            if base.exists() and base.is_dir():
                for p in base.iterdir():
                    if not p.is_file():
                        continue
                    ext = p.suffix.lower()
                    if ext not in (".json", ".txt"):
                        continue
                    try:
                        s = p.stat()
                        rows.append({
                            "path": str(p),
                            "size": int(s.st_size),
                            "mtime": float(s.st_mtime)
                        })
                    except Exception:
                        pass
            rows.sort(key=lambda r: r["mtime"], reverse=True)
        except Exception as e:
            self.view.setPlainText(f"Could not scan reports:\n{e}")
            return []
        return rows

    def refresh(self):
        self._items = self._scan_reports()
        self.list.clear()
        self.view.clear()
        if not self._items:
            self.view.setPlainText("No reports found.")
            return
        for it in self._items:
            p = Path(it["path"])
            dt = self._format_dt(it["mtime"])
            sz = it["size"]
            self.list.addItem(QListWidgetItem(f"{p.name}   ({sz} bytes, {dt})"))
        if self.list.count() > 0:
            self.list.setCurrentRow(0)

    def _on_select(self, cur: QListWidgetItem, prev: QListWidgetItem):
        idx = self.list.currentRow()
        if idx < 0 or idx >= len(self._items):
            self.view.clear()
            return
        path = Path(self._items[idx]["path"])
        if not path.exists():
            self.view.setPlainText("(file not found)")
            return
        try:
            txt = path.read_text(encoding="utf-8", errors="replace")
        except Exception as e:
            self.view.setPlainText(f"Could not read file:\n{e}")
            return
        if path.suffix.lower() == ".json":
            self.view.setPlainText(self._summarize_json(txt))
        else:
            self.view.setPlainText(txt)

    def open_dir(self):
        base = self._base_dir
        try:
            base.mkdir(parents=True, exist_ok=True)
            if base.exists():
                os.startfile(str(base))
            else:
                self.view.setPlainText(f"Folder does not exist:\n{base}")
        except Exception as e:
            self.view.setPlainText(f"Could not open folder:\n{e}")

    def _current_path(self) -> Path | None:
        idx = self.list.currentRow()
        if idx < 0 or idx >= len(self._items):
            return None
        p = Path(self._items[idx]["path"]) if self._items else None
        return p if p and p.exists() else None

    def open_file(self):
        p = self._current_path()
        if not p:
            return
        try:
            os.startfile(str(p))
        except Exception as e:
            self.view.setPlainText(f"Could not open file:\n{e}")

    def delete_selected(self):
        p = self._current_path()
        if not p:
            return
        try:
            p.unlink(missing_ok=True)
            self.refresh()
        except Exception as e:
            self.view.setPlainText(f"Could not delete file:\n{e}")

    def copy_preview(self):
        txt = self.view.toPlainText()
        try:
            QGuiApplication.clipboard().setText(txt)
        except Exception:
            pass