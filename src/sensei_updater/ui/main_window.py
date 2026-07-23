# sensei_updater/ui/main_window.py
from pathlib import Path
from importlib import resources
from PySide6.QtCore import Qt, QTimer, QSize
from PySide6.QtWidgets import QApplication, QMainWindow, QWidget, QHBoxLayout, QVBoxLayout, QPushButton, QLabel, QStyle, QMessageBox, QSizePolicy
from .widgets import Sidebar, PageStack, Header, GlassCard, MetricTile, ResponsiveGrid, std_icon
from .pages.cycle_page import CyclePage
from .pages.apps_page import Apps
from .pages.drivers_page import Drivers
from .pages.settings_page import SettingsPage
from .pages.schedule_page import SchedulePage
from .pages.reports_page import ReportsPage
from ..core.admin import is_admin, run_as_admin

def _sanitize_qss(text: str) -> str:
    return text.replace("text-visible", "qproperty-textVisible")

class TitleBar(QWidget):
    def __init__(self, parent: QMainWindow):
        super().__init__(parent)
        self.setObjectName("TitleBar")
        self._drag_pos = None
        h = QHBoxLayout(self)
        h.setContentsMargins(8, 6, 8, 6)
        h.setSpacing(8)
        self.lbl_title = QLabel("Sensei Updater", self)
        self.lbl_title.setStyleSheet("color:#e6eeff;font-weight:600")
        self.lbl_title.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Preferred)
        h.addWidget(self.lbl_title, 1)
        if not is_admin():
            self.btn_admin = QPushButton("Run as Admin", self)
            self.btn_admin.setObjectName("PrimaryButton")
            self.btn_admin.clicked.connect(lambda: run_as_admin())
            h.addWidget(self.btn_admin)
        self.btn_min = QPushButton("", self)
        self.btn_min.setObjectName("TitleBtn")
        self.btn_min.setProperty("btn", "min")
        self.btn_min.setIcon(QApplication.style().standardIcon(QStyle.SP_TitleBarMinButton))
        self.btn_min.setFlat(True)
        self.btn_min.clicked.connect(self.window().showMinimized)
        h.addWidget(self.btn_min)
        self.btn_max = QPushButton("", self)
        self.btn_max.setObjectName("TitleBtn")
        self.btn_max.setProperty("btn", "max")
        self.btn_max.setIcon(QApplication.style().standardIcon(QStyle.SP_TitleBarMaxButton))
        self.btn_max.setFlat(True)
        self.btn_max.clicked.connect(self._toggle_max_restore)
        h.addWidget(self.btn_max)
        self.btn_close = QPushButton("", self)
        self.btn_close.setObjectName("TitleBtn")
        self.btn_close.setProperty("btn", "close")
        self.btn_close.setIcon(QApplication.style().standardIcon(QStyle.SP_TitleBarCloseButton))
        self.btn_close.setFlat(True)
        self.btn_close.clicked.connect(self.window().close)
        h.addWidget(self.btn_close)

    def _toggle_max_restore(self):
        w = self.window()
        if w.isMaximized():
            w.showNormal()
        else:
            w.showMaximized()

    def mousePressEvent(self, e):
        if e.button() == Qt.LeftButton:
            self._drag_pos = e.globalPosition().toPoint() - self.window().frameGeometry().topLeft()
            e.accept()
        else:
            super().mousePressEvent(e)

    def mouseMoveEvent(self, e):
        if (e.buttons() & Qt.LeftButton) and self._drag_pos is not None and not self.window().isMaximized():
            self.window().move(e.globalPosition().toPoint() - self._drag_pos)
            e.accept()
        else:
            super().mouseMoveEvent(e)

    def mouseReleaseEvent(self, e):
        self._drag_pos = None
        super().mouseReleaseEvent(e)

class Dashboard(QWidget):
    def __init__(self, version_text: str, reboot_text: str,
                 on_apps, on_drivers, on_reports, on_cycle):
        super().__init__()
        v = QVBoxLayout(self)
        v.setContentsMargins(16, 16, 16, 16)
        v.setSpacing(12)
        v.addWidget(Header("Dashboard"))

        hero = GlassCard()
        hv = QHBoxLayout()
        hv.setSpacing(12)
        left = QVBoxLayout(); left.setSpacing(6)
        title = QLabel("Welcome to Sensei Updater")
        title.setStyleSheet("color:#e6eeff;font-size:20px;font-weight:800")
        subtitle = QLabel("Scan, update, and keep your system healthy in one place.")
        subtitle.setStyleSheet("color:#c9d3ee")
        left.addWidget(title)
        left.addWidget(subtitle)
        
        actions_h = QHBoxLayout(); actions_h.setSpacing(8)
        def mk_btn(text, icon, cb):
            b = QPushButton(text)
            b.setObjectName("PrimaryButton")
            b.setIcon(std_icon(icon))
            b.clicked.connect(cb)
            return b
        actions_h.addWidget(mk_btn("Manage Apps", QStyle.SP_DirIcon, on_apps))
        actions_h.addWidget(mk_btn("Driver Updates", QStyle.SP_ComputerIcon, on_drivers))
        actions_h.addWidget(mk_btn("Run Cycle", QStyle.SP_MediaPlay, on_cycle))
        actions_h.addWidget(mk_btn("Reports", QStyle.SP_FileDialogListView, on_reports))
        left.addLayout(actions_h)
        hv.addLayout(left, 1)
        hero.v.addLayout(hv)

        grid = ResponsiveGrid(min_col_w=220, max_cols=4)
        grid.add(MetricTile("Version", version_text))
        grid.add(MetricTile("Pending Reboot", reboot_text))

        v.addWidget(hero)
        v.addWidget(grid)

class MainWindow(QMainWindow):
    def __init__(self, console, app, drivers, system, cfg, sched, reports, version_text):
        super().__init__()
        self.console, self.app, self.drivers, self.system = console, app, drivers, system
        self.cfg, self.sched, self.reports = cfg, sched, reports
        self.setWindowFlag(Qt.FramelessWindowHint, True)
        self.setWindowTitle("Sensei Updater")
        self.resize(1220, 760)
        root = QWidget()
        self.setCentralWidget(root)
        outer = QVBoxLayout(root)
        outer.setContentsMargins(0, 0, 0, 0)
        self.titlebar = TitleBar(self)
        outer.addWidget(self.titlebar)
        body = QWidget()
        outer.addWidget(body, 1)
        h = QHBoxLayout(body)
        h.setContentsMargins(0, 0, 0, 0)
        self.sidebar = Sidebar()
        self.pages = PageStack()
        reboot = "Yes" if self._safe_pending_reboot() else "No"
        dash = QWidget()
        dv = QVBoxLayout(dash)
        dv.setContentsMargins(0, 0, 0, 0)
        cycle_page = CyclePage(app, drivers, system, cfg)
        self._cycle_page = cycle_page
        d_hdr = Dashboard(version_text, reboot, self.goto_apps, self.goto_drivers, self.goto_reports, self.start_cycle)
        dv.addWidget(d_hdr)
        apps = Apps(app)
        drivers_w = Drivers(drivers)
        reports_w = ReportsPage(cfg)
        settings_w = SettingsPage(cfg, sched, app)
        sched_w = SchedulePage(cfg, sched)
        for w in (dash, apps, drivers_w, cycle_page, sched_w, reports_w, settings_w):
            self.pages.addWidget(w)
        self.btn_dash = self.sidebar.add("Dashboard", self.goto_dash, True)
        self.btn_apps = self.sidebar.add("Apps", self.goto_apps)
        self.btn_drivers = self.sidebar.add("Drivers", self.goto_drivers)
        self.btn_cycle = self.sidebar.add("Cycle", self.goto_cycle)
        self.btn_schedule = self.sidebar.add("Schedule", self.goto_schedule)
        self.btn_reports = self.sidebar.add("Reports", self.goto_reports)
        self.btn_settings = self.sidebar.add("Settings", self.goto_settings)
        h.addWidget(self.sidebar)
        h.addWidget(self.pages, 1)
        try:
            self.pages.setCurrentIndex(0)
        except Exception:
            pass
        try:
            self.app.check_environment()
        except Exception:
            pass

    def _sel(self, idx, btn):
        self.sidebar.select(btn)
        self.pages.setCurrentIndexAnimated(idx)

    def goto_dash(self): self._sel(0, self.btn_dash)
    def goto_apps(self): self._sel(1, self.btn_apps)
    def goto_drivers(self): self._sel(2, self.btn_drivers)
    def goto_cycle(self): self._sel(3, self.btn_cycle)
    def goto_schedule(self): self._sel(4, self.btn_schedule)
    def goto_reports(self): self._sel(5, self.btn_reports)
    def goto_settings(self): self._sel(6, self.btn_settings)
    def start_cycle(self): self.goto_cycle(); self._cycle_page.run_all()

    def _safe_pending_reboot(self):
        try:
            return self.system.has_pending_reboot()
        except Exception:
            return False

def _load_qss_text(qss_path: str | None):
    if qss_path:
        p = Path(qss_path)
        if p.exists():
            try:
                return p.read_text(encoding="utf-8")
            except Exception:
                pass
    try:
        with resources.as_file(resources.files("sensei_updater.ui").joinpath("styles.qss")) as p:
            if p.exists():
                return p.read_text(encoding="utf-8")
    except Exception:
        pass
    local = Path(__file__).with_name("styles.qss")
    if local.exists():
        return local.read_text(encoding="utf-8")
    return ""

def _install_exception_guard():
    import sys, traceback
    def guard_hook(exc_type, exc, tb):
        msg = "".join(traceback.format_exception(exc_type, exc, tb))[-3000:]
        if QApplication.instance() is None:
            print(msg)
            return
        m = QMessageBox()
        m.setWindowTitle("Error")
        m.setText("An unexpected error occurred.")
        m.setDetailedText(msg)
        m.exec()
    sys.excepthook = guard_hook

def start_gui(console, app, drivers, system, cfg, sched, reports, version_text, qss_path=None):
    print("[GUI] init QApplication")
    QApplication.setAttribute(Qt.AA_UseHighDpiPixmaps, True)
    app_instance = QApplication.instance() or QApplication([])
    print("[GUI] install exception guard")
    _install_exception_guard()
    qss = _load_qss_text(qss_path)
    print(f"[GUI] load qss: {'ok' if bool(qss) else 'empty'}")
    if qss:
        app_instance.setStyleSheet(_sanitize_qss(qss))
    print("[GUI] build MainWindow")
    main_window = MainWindow(console, app, drivers, system, cfg, sched, reports, version_text)
    print("[GUI] show window")
    app_instance._main_window = main_window
    main_window.show()
    print("[GUI] entering event loop")
    rc = app_instance.exec()
    print(f"[GUI] event loop exited with code {rc}")
    return rc