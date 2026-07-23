from __future__ import annotations
from typing import List, Dict, Any
from PySide6.QtCore import Qt, QRectF, QSize, Signal, Slot, QTimer
from PySide6.QtGui import QPainter, QFont, QPen, QFontMetrics
from PySide6.QtWidgets import QWidget, QVBoxLayout, QHBoxLayout, QPushButton, QLabel, QListWidget, QListWidgetItem, QStackedLayout, QSizePolicy
from ..widgets import Header, GlassCard
from ..async_utils import BusyOverlay, JobController, run_async
from ...core.admin import is_admin

class _CircularProgress(QWidget):
    def __init__(self):
        super().__init__()
        self._value = 0
        self._text_top = ""
        self._text_bottom = ""
        self.setMinimumSize(220, 220)
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def sizeHint(self) -> QSize:
        return QSize(260, 260)

    def setValue(self, v: int):
        v = max(0, min(100, int(v)))
        if v != self._value:
            self._value = v
            self.update()

    def setTexts(self, top: str, bottom: str):
        if (top != self._text_top) or (bottom != self._text_bottom):
            self._text_top = top
            self._text_bottom = bottom
            self.update()

    def paintEvent(self, e):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing, True)
        r = self.rect()
        s = min(r.width(), r.height())
        d = s - 24
        cx, cy = r.center().x(), r.center().y()
        rect = QRectF(cx - d / 2, cy - d / 2, d, d)
        pen_bg = QPen()
        pen_bg.setWidth(12)
        pen_bg.setColor(self.palette().mid().color())
        p.setPen(pen_bg)
        p.drawArc(rect, 0, 360 * 16)
        pen_fg = QPen()
        pen_fg.setWidth(12)
        pen_fg.setColor(self.palette().highlight().color())
        p.setPen(pen_fg)
        span = int(360 * 16 * (self._value / 100.0))
        p.drawArc(rect, -90 * 16, -span)
        p.setPen(self.palette().text().color())
        f1 = QFont(self.font()); f1.setPointSize(int(d / 8)); f1.setBold(True)
        p.setFont(f1)
        p.drawText(r, Qt.AlignCenter, f"{self._value}%")
        f2 = QFont(self.font()); f2.setPointSize(int(d / 16))
        p.setFont(f2)
        ty = cy - int(d * 0.28)
        by = cy + int(d * 0.28)
        p.drawText(QRectF(r.left() + 12, ty - 20, r.width() - 24, 40), Qt.AlignCenter, self._text_top or "")
        p.drawText(QRectF(r.left() + 12, by - 20, r.width() - 24, 40), Qt.AlignCenter, self._text_bottom or "")

class Drivers(QWidget):
    progressChanged = Signal(int, str, str)

    def __init__(self, driver_service):
        super().__init__()
        self.drivers = driver_service
        self._last_pct = 0
        self.progressChanged.connect(self._on_progress_safe, Qt.QueuedConnection)
        root = QVBoxLayout(self); root.setContentsMargins(0, 0, 0, 0); root.setSpacing(0)
        root.addWidget(Header("Drivers"))
        card = GlassCard()
        top = QHBoxLayout(); top.setSpacing(8)
        self.btn_scan = QPushButton("Scan"); self.btn_scan.setObjectName("PrimaryButton")
        self.btn_install_sel = QPushButton("Update Selected"); self.btn_install_sel.setObjectName("PrimaryButton")
        self.btn_install_all = QPushButton("Update All"); self.btn_install_all.setObjectName("PrimaryButton")
        self.btn_select_all = QPushButton("Select All"); self.btn_select_all.setObjectName("PrimaryButton")
        self.btn_select_none = QPushButton("Select None"); self.btn_select_none.setObjectName("PrimaryButton")
        top.addWidget(self.btn_scan); top.addWidget(self.btn_install_sel); top.addWidget(self.btn_install_all); top.addWidget(self.btn_select_all); top.addWidget(self.btn_select_none); top.addStretch(1)
        self.stack = QStackedLayout()
        self.panel_list = QWidget()
        pl = QVBoxLayout(self.panel_list); pl.setContentsMargins(0, 0, 0, 0)
        self.msg = QLabel(""); self.msg.setObjectName("Chip")
        self.list = QListWidget()
        self.list.setAlternatingRowColors(True)
        self.list.setSelectionMode(QListWidget.NoSelection)
        self.list.setUniformItemSizes(True)
        self.list.setWordWrap(False)
        self.list.setTextElideMode(Qt.ElideMiddle)
        pl.addWidget(self.msg)
        pl.addWidget(self.list)
        self.panel_progress = QWidget()
        pp = QVBoxLayout(self.panel_progress); pp.setContentsMargins(0, 32, 0, 32); pp.setSpacing(8)
        self.circle = _CircularProgress()
        self.label_line1 = QLabel(""); self.label_line1.setAlignment(Qt.AlignCenter)
        self.label_line2 = QLabel(""); self.label_line2.setAlignment(Qt.AlignCenter)
        self.label_line1.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.label_line2.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        pp.addStretch(1); pp.addWidget(self.circle, 0, Qt.AlignCenter); pp.addSpacing(8)
        pp.addWidget(self.label_line1, 0, Qt.AlignCenter); pp.addWidget(self.label_line2, 0, Qt.AlignCenter); pp.addStretch(1)
        self.stack.addWidget(self.panel_list)
        self.stack.addWidget(self.panel_progress)
        card.v.addLayout(top)
        card.v.addLayout(self.stack)
        outer = QVBoxLayout(); outer.setContentsMargins(24, 24, 24, 24); outer.addWidget(card)
        root.addLayout(outer)
        self.overlay = BusyOverlay(self, compact=True)
        self.jobs = JobController(self, self.overlay)
        self.btn_scan.clicked.connect(self.scan)
        self.btn_install_all.clicked.connect(self.install_all)
        self.btn_install_sel.clicked.connect(self.install_selected)
        self.btn_select_all.clicked.connect(self.select_all)
        self.btn_select_none.clicked.connect(self.select_none)

    def _elide(self, text: str) -> str:
        fm = QFontMetrics(self.label_line1.font())
        w = max(0, self.panel_progress.width() - 48)
        return fm.elidedText(text or "", Qt.ElideMiddle, w)

    def resizeEvent(self, e):
        self.label_line1.setText(self._elide(self.label_line1.text()))
        self.label_line2.setText(self._elide(self.label_line2.text()))
        super().resizeEvent(e)

    def _populate(self, rows: List[Dict[str, Any]]):
        self.list.clear()
        if not rows:
            it = QListWidgetItem("No available updates"); it.setFlags(Qt.NoItemFlags)
            self.list.addItem(it)
            self.msg.setText("No available updates")
            return
        for r in rows:
            kb = r.get("kb") or ""
            title = r.get("title") or ""
            txt = (f"{kb}  {title}").strip()
            it = QListWidgetItem(txt)
            it.setFlags(it.flags() | Qt.ItemIsUserCheckable | Qt.ItemIsEnabled)
            it.setCheckState(Qt.Unchecked)
            it.setData(Qt.UserRole, kb)
            self.list.addItem(it)
        self.msg.setText(f"Found {len(rows)} driver updates")

    @Slot(int, str, str)
    def _on_progress_safe(self, p: int, line1: str, line2: str):
        p = max(self._last_pct, max(0, min(100, int(p))))
        self._last_pct = p
        self.circle.setValue(p)
        a = self._elide(line1)
        b = self._elide(line2)
        self.circle.setTexts(a or "", b or "")
        self.label_line1.setText(a or "")
        self.label_line2.setText(b or "")

    def _show_list(self):
        idx = self.stack.indexOf(self.panel_list)
        QTimer.singleShot(0, lambda: self.stack.setCurrentIndex(idx))

    def _show_progress(self):
        idx = self.stack.indexOf(self.panel_progress)
        QTimer.singleShot(0, lambda: self.stack.setCurrentIndex(idx))

    def scan(self):
        self.msg.setText("")
        self._last_pct = 0
        self.circle.setValue(0)
        self.circle.setTexts("Preparing", "")
        self.label_line1.setText("")
        self.label_line2.setText("")
        self._show_progress()

        def task(progress, message):
            self.progressChanged.emit(10, "Contacting Windows Update", "Drivers")
            message("Scanning drivers…")
            try:
                rows = self.drivers.list_available(timeout_sec=300) or []
            except Exception as e:
                return {"error": str(e)}
            progress(100)
            self.progressChanged.emit(100, "Done", "")
            return rows

        def done(res):
            self._last_pct = 100
            self.circle.setValue(100)
            if isinstance(res, dict) and res.get("error"):
                self._populate([])
                self.list.addItem(QListWidgetItem(f"Scan error: {res['error']}"))
                self.msg.setText(f"Scan error: {res['error']}")
                self._show_list()
                return
            self._populate(res)
            self._show_list()

        t, w = run_async(task)
        self.jobs.start(t, w, "Scanning drivers…",
                        [self.btn_scan, self.btn_install_all, self.btn_install_sel],
                        done, timeout_ms=300000)

    def _checked_kbs(self) -> List[str]:
        out: List[str] = []
        for i in range(self.list.count()):
            it = self.list.item(i)
            if it and it.checkState() == Qt.Checked:
                kb = it.data(Qt.UserRole)
                if kb:
                    out.append(kb)
        return out

    def select_all(self):
        for i in range(self.list.count()):
            it = self.list.item(i)
            if it and (it.flags() & Qt.ItemIsUserCheckable):
                it.setCheckState(Qt.Checked)
        self.msg.setText("")

    def select_none(self):
        for i in range(self.list.count()):
            it = self.list.item(i)
            if it and (it.flags() & Qt.ItemIsUserCheckable):
                it.setCheckState(Qt.Unchecked)
        self.msg.setText("")

    def install_selected(self):
        if not is_admin():
            self.msg.setText("Administrator required for driver installation")
            return
        kbs = self._checked_kbs()
        if not kbs:
            self.msg.setText("No items selected")
            return

        def task(progress, message):
            message("Installing selected drivers…")
            try:
                ok, reboot = self.drivers.install_kbs(kbs)
            except Exception as e:
                return {"error": str(e)}
            return {"ok": ok, "reboot": reboot}

        def done(res):
            if isinstance(res, dict) and res.get("error"):
                self.msg.setText(f"Failed: {res['error']}")
                return
            self.msg.setText("Reboot required" if res.get("reboot") else ("Done" if res.get("ok") else "Failed"))
            self.scan()

        t, w = run_async(task)
        self.jobs.start(t, w, "Installing drivers…",
                        [self.btn_scan, self.btn_install_all, self.btn_install_sel],
                        done, timeout_ms=900000)

    def install_all(self):
        if not is_admin():
            self.msg.setText("Administrator required for driver installation")
            return

        def task(progress, message):
            message("Installing available drivers…")
            try:
                ok, reboot = self.drivers.update_drivers()
            except Exception as e:
                return {"error": str(e)}
            return {"ok": ok, "reboot": reboot}

        def done(res):
            if isinstance(res, dict) and res.get("error"):
                self.msg.setText(f"Failed: {res['error']}")
                return
            self.msg.setText("Reboot required" if res.get("reboot") else ("Done" if res.get("ok") else "Failed"))
            self.scan()

        t, w = run_async(task)
        self.jobs.start(t, w, "Installing drivers…",
                        [self.btn_scan, self.btn_install_all, self.btn_install_sel],
                        done, timeout_ms=900000)