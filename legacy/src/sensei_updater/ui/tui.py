from typing import List, Dict
from textual.app import App, ComposeResult
from textual.widgets import Header, Footer, DataTable, Static, Button, Input
from textual.containers import Vertical, Horizontal
from textual.worker import Worker
from textual.message import Message

class UpgradesLoaded(Message):
    def __init__(self, rows: List[Dict]):
        super().__init__()
        self.rows = rows

class UpdateFinished(Message):
    def __init__(self, ok_ids: List[str]):
        super().__init__()
        self.ok_ids = ok_ids

class UpdaterTUI(App):
    CSS = """
    Screen {align: center middle}
    #title {content-align: center; padding: 1 0}
    #controls {padding: 0 1; height: auto}
    #table {height: 1fr; width: 100%}
    #status {padding: 0 1}
    """

    def __init__(self, console, app_service, cfg):
        super().__init__()
        self.console = console
        self.app_service = app_service
        self.cfg = cfg
        self.rows: List[Dict] = []
        self.selected_ids: set[str] = set()
        self._scan_worker: Worker | None = None
        self._update_worker: Worker | None = None

    def compose(self) -> ComposeResult:
        yield Header()
        yield Static("Sensei's Updater — TUI", id="title")
        with Horizontal(id="controls"):
            yield Button("Refresh", id="refresh")
            yield Button("Toggle", id="toggle")
            yield Button("Update Selected", id="update")
            yield Button("Use Default Profile", id="profile")
            yield Input(placeholder="Filter text", id="filter")
            yield Button("Apply Filter", id="apply_filter")
            yield Button("Quit", id="quit")
        self.table = DataTable(id="table", cursor_type="row")
        self.table.add_columns("Sel","Name","Id","Installed","Available","Source")
        yield self.table
        yield Static("", id="status")
        yield Footer()

    def on_mount(self):
        self.refresh_table()

    def set_status(self, text: str):
        s = self.query_one("#status", Static)
        s.update(text)

    def refresh_table(self):
        if self._scan_worker and not self._scan_worker.is_finished:
            return
        self.set_status("Scanning for app updates…")
        self._scan_worker = self.run_worker(self._load_rows(), exclusive=True, group="scan", name="scan")

    async def _load_rows(self):
        rows = self.app_service.list_upgrades() or []
        if not rows:
            rows = self.app_service.list_installed() or []
        await self.post_message(UpgradesLoaded(rows))

    def on_upgrades_loaded(self, msg: UpgradesLoaded):
        self.rows = msg.rows or []
        self.rebuild_table(self.rows)
        if not self.rows:
            self.set_status("No apps found.")
        else:
            self.set_status(f"Loaded {len(self.rows)} rows.")

    def rebuild_table(self, data: List[Dict]):
        self.table.clear()
        for r in data:
            pid = r.get("Id","")
            sel = "✔" if pid in self.selected_ids else ""
            self.table.add_row(sel, r.get("Name",""), pid, r.get("Version",""), r.get("Available",""), r.get("Source",""))

    def action_toggle_current(self):
        if self.table.row_count == 0:
            return
        row = self.table.cursor_row or 0
        pid = self.rows[row].get("Id","")
        if not pid:
            return
        if pid in self.selected_ids:
            self.selected_ids.remove(pid)
        else:
            self.selected_ids.add(pid)
        self.rebuild_table(self.rows)

    def on_button_pressed(self, event):
        bid = event.button.id
        if bid == "refresh":
            self.refresh_table()
        elif bid == "toggle":
            self.action_toggle_current()
        elif bid == "profile":
            d = self.cfg.get_defaults()
            pn = d.get("profile")
            if pn:
                ids = list(self.cfg.get_profile(pn))
                if ids:
                    self.set_status(f"Updating profile '{pn}'…")
                    self._update_worker = self.run_worker(self._do_update(ids), exclusive=True, group="update", name="update")
        elif bid == "update":
            ids = list(self.selected_ids)
            if ids:
                self.set_status(f"Updating {len(ids)} package(s)…")
                self._update_worker = self.run_worker(self._do_update(ids), exclusive=True, group="update", name="update")
        elif bid == "apply_filter":
            term = self.query_one("#filter", Input).value.strip().lower()
            if not term:
                self.rebuild_table(self.rows)
                self.set_status(f"{len(self.rows)} rows.")
                return
            filt = []
            for r in self.rows:
                if term in (r.get("Name","").lower() + " " + r.get("Id","").lower()):
                    filt.append(r)
            self.rebuild_table(filt)
            self.set_status(f"Filtered to {len(filt)} rows.")
        elif bid == "quit":
            self.exit()

    async def _do_update(self, ids: List[str]):
        res = self.app_service.update_ids(ids)
        ok = list(res.get("updated", [])) + list(res.get("interactive", [])) + list(res.get("reinstalled", []))
        await self.post_message(UpdateFinished(ok))

    def on_update_finished(self, msg: UpdateFinished):
        for pid in msg.ok_ids:
            if pid in self.selected_ids:
                self.selected_ids.remove(pid)
        self.refresh_table()

def run_tui(console, app_service, cfg):
    app = UpdaterTUI(console, app_service, cfg)
    app.run()