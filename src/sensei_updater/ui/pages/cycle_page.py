from __future__ import annotations
from typing import Dict, Any, List
import os
from pathlib import Path
from datetime import datetime
from PySide6.QtCore import Qt, QRect, QEasingCurve, QPropertyAnimation, QPoint, Signal
from PySide6.QtWidgets import QWidget, QVBoxLayout, QHBoxLayout, QPushButton, QLabel, QListWidget, QListWidgetItem, QFrame, QStackedLayout, QTextEdit, QScrollArea, QSizePolicy
from ..widgets import Header, GlassCard
from ..async_utils import BusyOverlay, JobController, run_async
from ...domain.reports import RunReport

def _expand_env(path_str: str) -> Path:
    try:
        return Path(os.path.expandvars(path_str)).expanduser()
    except Exception:
        return Path(path_str).expanduser()

class StepItem(QFrame):
    def __init__(self, text: str):
        super().__init__()
        self.setFixedHeight(36)
        h = QHBoxLayout(self)
        h.setContentsMargins(12, 0, 12, 0)
        h.setSpacing(8)
        self.dot = QLabel("●")
        self.dot.setStyleSheet("color:#444;font-weight:900;")
        self.label = QLabel(text)
        self.label.setStyleSheet("color:#cfd6eb;")
        h.addWidget(self.dot)
        h.addWidget(self.label)
        h.addStretch()

class Stepper(QWidget):
    def __init__(self, steps: List[str]):
        super().__init__()
        self._steps: List[StepItem] = []
        self._marker = QFrame(self)
        self._marker.setStyleSheet("background:#FF9F1C;border-radius:6px;")
        self._marker.setGeometry(QRect(0, 0, 4, 24))
        v = QVBoxLayout(self)
        v.setContentsMargins(0, 0, 0, 0)
        v.setSpacing(8)
        for s in steps:
            item = StepItem(s)
            self._steps.append(item)
            v.addWidget(item)
        v.addStretch(1)
        self._anim = QPropertyAnimation(self._marker, b"pos", self)
        self._anim.setDuration(220)
        self._anim.setEasingCurve(QEasingCurve.InOutQuad)
        self.setCurrent(0, instant=True)

    def setCurrent(self, idx: int, instant: bool = False):
        idx = max(0, min(idx, len(self._steps) - 1))
        for i, it in enumerate(self._steps):
            if i < idx:
                it.dot.setText("✓")
                it.dot.setStyleSheet("color:#6bd66b;font-weight:900;")
                it.label.setStyleSheet("color:#9fd3a9;")
            elif i == idx:
                it.dot.setText("●")
                it.dot.setStyleSheet("color:#FF9F1C;font-weight:900;")
                it.label.setStyleSheet("color:#FFE58A;font-weight:700;")
            else:
                it.dot.setText("●")
                it.dot.setStyleSheet("color:#444;font-weight:900;")
                it.label.setStyleSheet("color:#cfd6eb;")
        target_y = self._steps[idx].y() + (self._steps[idx].height() - 24) // 2
        dest = QPoint(0, target_y)
        if instant:
            self._marker.move(dest)
        else:
            self._anim.stop()
            self._anim.setStartValue(self._marker.pos())
            self._anim.setEndValue(dest)
            self._anim.start()

class _MiniCircle(QWidget):
    def __init__(self):
        super().__init__()
        self._value = 0
        self._t1 = ""
        self._t2 = ""
        self.setMinimumSize(200, 200)
        self.setSizePolicy(QSizePolicy.Fixed, QSizePolicy.Fixed)

    def setValue(self, v: int):
        v = max(0, min(100, int(v)))
        if v != self._value:
            self._value = v
            self.update()

    def setTexts(self, t1: str, t2: str):
        if self._t1 != t1 or self._t2 != t2:
            self._t1, self._t2 = t1, t2
            self.update()

    def paintEvent(self, e):
        from PySide6.QtGui import QPainter, QPen, QFont
        from PySide6.QtCore import QRectF, Qt
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing, True)
        r = self.rect()
        s = min(r.width(), r.height())
        d = s - 24
        cx, cy = r.center().x(), r.center().y()
        rect = QRectF(cx - d / 2, cy - d / 2, d, d)
        pen_bg = QPen(); pen_bg.setWidth(10); pen_bg.setColor(self.palette().mid().color())
        p.setPen(pen_bg); p.drawArc(rect, 0, 360 * 16)
        pen_fg = QPen(); pen_fg.setWidth(10); pen_fg.setColor(self.palette().highlight().color())
        p.setPen(pen_fg)
        span = int(360 * 16 * (self._value / 100.0))
        p.drawArc(rect, -90 * 16, -span)
        p.setPen(self.palette().text().color())
        f1 = QFont(self.font()); f1.setPointSize(int(d / 7)); f1.setBold(True)
        p.setFont(f1); p.drawText(r, Qt.AlignCenter, f"{self._value}%")
        f2 = QFont(self.font()); f2.setPointSize(int(d / 14))
        p.setFont(f2)
        p.drawText(QRectF(r.left(), r.top() + 8, r.width(), 24), Qt.AlignCenter, self._t1)
        p.drawText(QRectF(r.left(), r.bottom() - 32, r.width(), 24), Qt.AlignCenter, self._t2)

class CyclePage(QWidget):
    def __init__(self, app_service, driver_service, system_service, cfg):
        super().__init__()
        self.app = app_service
        self.drivers = driver_service
        self.system = system_service
        self.cfg = cfg
        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)
        root.addWidget(Header("Cycle"))
        self.stack = QStackedLayout()
        run_panel = QWidget()
        rv = QVBoxLayout(run_panel)
        wrap = GlassCard()
        flow = QHBoxLayout()
        flow.setSpacing(16)
        self.stepper = Stepper([
            "Prepare",
            "Scan Drivers",
            "Install Drivers",
            "Scan Apps",
            "Update Apps",
            "Cleanup",
            "Health Check",
            "Finish"
        ])
        flow.addWidget(self.stepper, 0)
        mid = QVBoxLayout()
        mid.setSpacing(8)
        self.status = QLabel("Ready")
        self.status.setObjectName("Chip")
        self.out = QListWidget()
        ctrl = QHBoxLayout()
        ctrl.setSpacing(8)
        self.btn_run = QPushButton("Run Full Cycle")
        self.btn_run.setObjectName("PrimaryButton")
        self.btn_rescan = QPushButton("Rescan Only")
        self.btn_rescan.setObjectName("PrimaryButton")
        ctrl.addWidget(self.btn_run)
        ctrl.addWidget(self.btn_rescan)
        ctrl.addStretch(1)
        self.inline_card = GlassCard()
        self.inline_text = QTextEdit()
        self.inline_text.setReadOnly(True)
        self.inline_finish = QPushButton("Finish")
        self.inline_finish.setObjectName("PrimaryButton")
        self.inline_card.v.addWidget(self.inline_text)
        self.inline_card.v.addWidget(self.inline_finish, 0, Qt.AlignRight)
        self.inline_card.setMaximumWidth(720)
        self.inline_card.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Maximum)
        self.inline_wrap = QWidget()
        iw = QVBoxLayout(self.inline_wrap)
        iw.setContentsMargins(0, 0, 0, 0)
        iw.addStretch(1)
        iw.addWidget(self.inline_card, 0, Qt.AlignHCenter)
        iw.addStretch(1)
        self.inline_card.hide()
        self.mid_stack = QStackedLayout()
        self.mid_stack.addWidget(self.out)
        self.mid_stack.addWidget(self.inline_wrap)
        mid.addWidget(self.status)
        mid.addLayout(self.mid_stack, 1)
        mid.addLayout(ctrl)
        right = QVBoxLayout()
        right.setSpacing(8)
        self.circle = _MiniCircle()
        self.c_line1 = QLabel("")
        self.c_line1.setAlignment(Qt.AlignCenter)
        self.c_line2 = QLabel("")
        self.c_line2.setAlignment(Qt.AlignCenter)
        right.addStretch(1)
        right.addWidget(self.circle, 0, Qt.AlignRight)
        right.addSpacing(6)
        right.addWidget(self.c_line1, 0, Qt.AlignRight)
        right.addWidget(self.c_line2, 0, Qt.AlignRight)
        right.addStretch(1)
        flow.addLayout(mid, 1)
        flow.addLayout(right, 0)
        wrap.v.addLayout(flow)
        pad = QVBoxLayout()
        pad.setContentsMargins(24, 24, 24, 24)
        pad.addWidget(wrap)
        rv.addLayout(pad, 1)
        report_panel = QWidget()
        repv = QVBoxLayout(report_panel)
        rep_card = GlassCard()
        self.rep_text = QTextEdit()
        self.rep_text.setReadOnly(True)
        self.btn_finish = QPushButton("Finish")
        self.btn_finish.setObjectName("PrimaryButton")
        rep_card.v.addWidget(self.rep_text)
        rep_card.v.addWidget(self.btn_finish, 0, Qt.AlignRight)
        rep_pad = QVBoxLayout()
        rep_pad.setContentsMargins(24, 24, 24, 24)
        rep_pad.addWidget(rep_card)
        repv.addLayout(rep_pad, 1)
        self.stack.addWidget(run_panel)
        self.stack.addWidget(report_panel)
        root.addLayout(self.stack, 1)
        self.overlay = BusyOverlay(self, compact=True)
        self.jobs = JobController(self, self.overlay)
        self.btn_run.clicked.connect(self.run_all)
        self.btn_rescan.clicked.connect(self.scan_only)
        self.btn_finish.clicked.connect(self._back_to_run)
        self.inline_finish.clicked.connect(self._inline_finish)
        self._last_pct = 0

    def _append(self, text: str):
        self.out.addItem(QListWidgetItem(text))
        self.out.scrollToBottom()

    def _set_circle(self, p: int, l1: str = "", l2: str = ""):
        p = max(self._last_pct, max(0, min(100, int(p))))
        self._last_pct = p
        self.circle.setValue(p)
        self.c_line1.setText(l1 or "")
        self.c_line2.setText(l2 or "")

    def _bump(self, step_idx: int, msg: str, progress_emit, msg_emit, pval: int):
        self.stepper.setCurrent(step_idx, instant=False)
        msg_emit(msg)
        progress_emit(pval)
        self._set_circle(pval, msg, "")

    def _resolve_reports_dir(self) -> Path:
        d = self.cfg.get_defaults() or {}
        out = d.get("out") or r"%LOCALAPPDATA%\SenseiUpdater\last-run.json"
        p = _expand_env(out)
        return p.parent if p.suffix else p

    def _save_report(self, rep: RunReport, d: Dict[str, Any]):
        rep.driver_success = bool(d.get("drivers", {}).get("installed"))
        rep.reboot_required = bool(d.get("drivers", {}).get("reboot"))
        for x in d.get("apps", {}).get("updated", []):
            rep.updated.append(x)
        for x in d.get("apps", {}).get("reinstalled", []):
            rep.reinstalled.append(x)
        for x in d.get("apps", {}).get("failed", []):
            rep.failed.append(x)
        for x in d.get("apps", {}).get("skipped", []):
            rep.skipped.append(x)
        for x in d.get("apps", {}).get("store_skipped", []):
            rep.store_skipped.append(x)
        for n in d.get("notes", []):
            rep.notes.append(n)
        rep.mark_finished()
        base = self._resolve_reports_dir()
        ts = datetime.now().strftime("%Y%m%d-%H%M%S")
        base.mkdir(parents=True, exist_ok=True)
        json_path = base / f"run-{ts}.json"
        txt_path = base / f"run-{ts}.txt"
        rep.save("json", json_path)
        rep.save("txt", txt_path)
        return str(json_path), str(txt_path)

    def _format_report_text(self, data: Dict[str, Any], paths: List[str] | tuple) -> str:
        lines = []
        lines.append("=== Sensei Updater Cycle Report ===")
        lines.append("")
        s = data.get("scanned", {})
        lines.append(f"Scanned Drivers: {s.get('drivers',0)}")
        lines.append(f"Scanned Apps: {s.get('apps',0)}")
        lines.append("")
        d = data.get("drivers", {})
        lines.append(f"Drivers Installed: {bool(d.get('installed'))}")
        lines.append(f"Reboot Required: {bool(d.get('reboot'))}")
        lines.append("")
        a = data.get("apps", {})
        lines.append(f"Apps Updated: {len(a.get('updated',[]))}")
        if a.get("updated"):
            for x in a["updated"]:
                lines.append(f"  + {x}")
        lines.append(f"Apps Reinstalled: {len(a.get('reinstalled',[]))}")
        if a.get("reinstalled"):
            for x in a["reinstalled"]:
                lines.append(f"  r {x}")
        lines.append(f"Apps Failed: {len(a.get("failed",[]))}")
        if a.get("failed"):
            for x in a["failed"]:
                lines.append(f"  ! {x}")
        lines.append(f"Apps Skipped: {len(a.get('skipped',[]))}")
        if a.get("skipped"):
            for x in a["skipped"]:
                lines.append(f"  ~ {x}")
        lines.append(f"Store Skipped: {len(a.get('store_skipped',[]))}")
        lines.append("")
        c = data.get("cleanup", {})
        lines.append(f"Cleanup OK: {bool(c.get('ok'))}")
        h = data.get("health", {})
        lines.append(f"Health Check OK: {bool(h.get('ok'))}")
        lines.append("")
        if data.get("notes"):
            lines.append("Notes:")
            for n in data["notes"]:
                lines.append(f"  - {n}")
            lines.append("")
        if paths:
            lines.append("Saved Files:")
            for p in paths:
                lines.append(f"  {p}")
        return "\n".join(lines)

    def _show_report(self, data: Dict[str, Any], paths: List[str] | tuple):
        txt = self._format_report_text(data, paths)
        self.rep_text.setPlainText(txt)
        self.stack.setCurrentIndex(1)

    def _show_inline_report(self, data: Dict[str, Any], paths: List[str] | tuple):
        txt = self._format_report_text(data, paths)
        self.inline_text.setPlainText(txt)
        self.inline_card.show()
        self.mid_stack.setCurrentWidget(self.inline_wrap)

    def _inline_finish(self):
        self.inline_text.clear()
        self.inline_card.hide()
        self._back_to_run()

    def _back_to_run(self):
        self.rep_text.clear()
        self.out.clear()
        self.status.setText("Ready")
        self._last_pct = 0
        self.circle.setValue(0)
        self.c_line1.setText("")
        self.c_line2.setText("")
        self.stack.setCurrentIndex(0)
        if hasattr(self, "mid_stack"):
            self.mid_stack.setCurrentWidget(self.out)

    def scan_only(self):
        def task(progress, message):
            res: Dict[str, Any] = {"drivers": [], "apps": [], "notes": []}
            d = self.cfg.get_defaults() or {}
            force = bool(d.get("force_refresh"))
            skip_store = bool(d.get("skip_store_scan", True))
            scan_to = int(d.get("scan_timeout_sec", 120))
            self._bump(0, "Preparing to scan…", progress, message, 5)
            try:
                self._bump(1, "Scanning drivers…", progress, message, 20)
                drv = self.drivers.list_available(timeout_sec=max(180, scan_to)) or []
            except Exception as e:
                drv = []
                res["notes"].append(f"drivers_scan_error: {e}")
            try:
                self._bump(3, "Scanning apps…", progress, message, 60)
                apps = self.app.list_upgrades_all(force_refresh=force, skip_store=skip_store) or []
            except Exception as e:
                apps = []
                res["notes"].append(f"apps_scan_error: {e}")
            res["drivers"] = drv
            res["apps"] = apps
            self._bump(7, "Done", progress, message, 100)
            return res

        def done(res):
            self.out.clear()
            if isinstance(res, dict) and res.get("error"):
                self.status.setText("Scan error")
                self._append(f"Scan error: {res['error']}")
                return
            drv = res.get("drivers", [])
            apps = res.get("apps", [])
            self.status.setText(f"Drivers: {len(drv)}  Apps: {len(apps)}")
            self._append(f"Scan complete. Drivers={len(drv)} Apps={len(apps)}")

        t, w = run_async(task)
        self.jobs.start(t, w, "Scanning…", [self.btn_run, self.btn_rescan], done, timeout_ms=360000)

    def run_all(self):
        def task(progress, message):
            out: Dict[str, Any] = {
                "scanned": {"drivers": 0, "apps": 0},
                "drivers": {"installed": False, "reboot": False},
                "apps": {"updated": [], "reinstalled": [], "failed": [], "skipped": [], "store_skipped": []},
                "cleanup": {"ok": False},
                "health": {"ok": False},
                "notes": []
            }
            rep = RunReport()
            d = self.cfg.get_defaults() or {}
            force = bool(d.get("force_refresh"))
            skip_store = bool(d.get("skip_store_scan", True))
            scan_to = int(d.get("scan_timeout_sec", 240))
            self._bump(0, "Preparing…", progress, message, 5)
            try:
                self._bump(1, "Scanning drivers…", progress, message, 15)
                drv_rows = self.drivers.list_available(timeout_sec=max(240, scan_to)) or []
            except Exception as e:
                drv_rows = []
                out["notes"].append(f"drivers_scan_error: {e}")
            out["scanned"]["drivers"] = len(drv_rows)
            if drv_rows:
                try:
                    self._bump(2, "Installing drivers…", progress, message, 35)
                    ok, reboot = self.drivers.update_drivers()
                    out["drivers"]["installed"] = bool(ok)
                    out["drivers"]["reboot"] = bool(reboot)
                except Exception as e:
                    out["drivers"]["installed"] = False
                    out["notes"].append(f"drivers_install_error: {e}")
            self._bump(3, "Scanning apps…", progress, message, 55)
            try:
                app_rows = self.app.list_upgrades_all(force_refresh=force, skip_store=skip_store) or []
            except Exception as e:
                app_rows = []
                out["notes"].append(f"apps_scan_error: {e}")
            out["scanned"]["apps"] = len(app_rows)
            if app_rows:
                try:
                    self._bump(4, "Updating apps…", progress, message, 75)
                    ids = [r.get("Id") for r in app_rows if r.get("Id")]
                    r = self.app.update_ids(ids)
                    for k in ("updated", "reinstalled", "failed", "skipped", "store_skipped"):
                        out["apps"][k] = list(r.get(k, []))
                except Exception as e:
                    out["notes"].append(f"apps_update_error: {e}")
            self._bump(5, "Cleaning up…", progress, message, 88)
            try:
                self.system.cleanup_temp()
                self.system.empty_recycle_bin()
                out["cleanup"]["ok"] = True
            except Exception as e:
                out["cleanup"]["ok"] = False
                out["notes"].append(f"cleanup_error: {e}")
            self._bump(6, "Health check…", progress, message, 94)
            try:
                self.system.dism_sfc()
                out["health"]["ok"] = True
            except Exception as e:
                out["health"]["ok"] = False
                out["notes"].append(f"health_error: {e}")
            try:
                if self.system.has_pending_reboot():
                    out["drivers"]["reboot"] = True or out["drivers"]["reboot"]
            except Exception:
                pass
            paths = self._save_report(rep, out)
            out["report_paths"] = paths
            self._bump(7, "Finish", progress, message, 100)
            return out

        def done(res):
            self.out.clear()
            if isinstance(res, dict) and res.get("error"):
                self.status.setText("Cycle error")
                self._append(f"Cycle error: {res['error']}")
                return
            self.status.setText("Finished")
            paths = res.get("report_paths") or ()
            self._show_inline_report(res, paths)

        t, w = run_async(task)
        self.jobs.start(t, w, "Running full cycle…", [self.btn_run, self.btn_rescan], done, timeout_ms=1440000)