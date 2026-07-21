from ..core.colors import *
from ..core.console import Console

class Selector:
    def __init__(self, console: Console, cfg):
        self.console = console
        self.cfg = cfg

    def print_table(self, pkgs, selected=None, title="Upgradable Apps (winget)"):
        selected = selected or set()
        self.console.header(title)
        if not pkgs:
            self.console.warn("No entries found.")
            return
        w = len(str(len(pkgs)))
        print(f"{ORANGE1}{BOLD}{'#'.rjust(w)}  {'Sel':3}  {'Name':40}  {'Id':34}  {'Installed':>12}  {'Available':>12}  {'Src':4}{RESET}")
        for i,p in enumerate(pkgs,1):
            sel = "✔" if p["Id"] in selected else " "
            name=(p.get("Name",""))[:40].ljust(40)
            pid =(p.get("Id",""))[:34].ljust(34)
            ver =(p.get("Version",""))[:12].rjust(12)
            ava =(p.get("Available",""))[:12].rjust(12)
            src =(p.get("Source",""))[:4].ljust(4)
            color = GREEN if sel=="✔" else GRAY
            print(f"{SUN}{str(i).rjust(w)}{RESET}   {color}{sel:3}{RESET}  {WHITE}{name}{RESET}  {GRAY}{pid}{RESET}  {ver}  {ava}  {src}")

    def loop(self, pkgs, app_service, title):
        selected = set()
        while True:
            self.console.pixel_art()
            self.print_table(pkgs, selected, title)
            print()
            print(f"{ORANGE1}{BOLD}Commands{RESET}: numbers (e.g. 1,3,5) | all | none |")
            print("  filter <text>      — show rows containing text")
            print("  search <text>      — search winget catalog; then use 'add <id>'")
            print("  add <id>           — add package id to selection")
            print("  rm <id>            — remove package id from selection")
            print("  u <id>             — update a single id immediately")
            print("  u all              — update all currently selected")
            print("  load <name>        — load saved selection")
            print("  save <name>        — save current selection")
            print("  profiles           — list saved profiles")
            print("  go                 — proceed to update selected")
            print("  back               — return to main menu")
            cmd = input(f"{ORANGE2}{BOLD}Select → {RESET}").strip()

            if not cmd: continue
            if cmd=="back": return None
            if cmd=="all":
                selected = {p["Id"] for p in pkgs if p["Id"]}
                self.console.ok(f"Selected ALL ({len(selected)})"); continue
            if cmd=="none":
                selected.clear(); self.console.ok("Cleared selection."); continue
            if cmd.startswith("filter "):
                q = cmd[7:].strip().lower()
                flt = [p for p in pkgs if q in (p.get("Name","").lower()) or q in (p.get("Id","").lower())]
                self.print_table(flt, selected, title=f"Filtered: '{q}'"); continue
            if cmd.startswith("search "):
                q = cmd[7:].strip()
                rc,out = app_service.proc.run_capture(["winget","search",q])
                if rc==0 and out:
                    rows = app_service._parse_table(out)
                    if rows:
                        self.print_table(rows, title="Search results (winget catalog)")
                        print(f"{GRAY}Tip: use 'add <id>' to add any of these ids to your selection.{RESET}")
                    else:
                        self.console.warn("No search results.")
                else:
                    self.console.warn("Search failed.")
                continue
            if cmd.startswith("add "):
                pid = cmd[4:].strip()
                if pid:
                    selected.add(pid); self.console.ok(f"Added: {pid}")
                continue
            if cmd.startswith("rm "):
                pid = cmd[3:].strip()
                if pid in selected:
                    selected.remove(pid); self.console.ok(f"Removed: {pid}")
                continue
            if cmd=="profiles":
                names = self.cfg.list_profiles()
                if not names: self.console.warn("No saved profiles.")
                else: self.console.info("Profiles: " + ", ".join(names))
                continue
            if cmd.startswith("load "):
                name = cmd[5:].strip()
                sel = self.cfg.get_profile(name)
                if not sel: self.console.warn(f"No saved profile named '{name}'.")
                else:
                    selected = set(sel); self.console.ok(f"Loaded profile '{name}' with {len(selected)} entries.")
                continue
            if cmd.startswith("save "):
                name = cmd[5:].strip()
                if not name: self.console.warn("Please provide a profile name.")
                else:
                    self.cfg.set_profile(name, selected); self.console.ok(f"Saved profile '{name}' with {len(selected)} entries.")
                continue
            if cmd.startswith("u "):
                arg = cmd[2:].strip()
                if arg=="all": app_service.update_ids(list(selected))
                else:
                    if not arg: self.console.warn("Usage: u <id>  |  u all")
                    else: app_service.update_ids([arg])
                continue
            if cmd=="go":
                if not selected: self.console.warn("No packages selected."); continue
                return list(selected)
            # indices
            try:
                idxs = [int(x.strip()) for x in cmd.split(",") if x.strip().isdigit()]
                changed=0
                for i in idxs:
                    if 1<=i<=len(pkgs):
                        pid = pkgs[i-1]["Id"]
                        if pid in selected: selected.remove(pid)
                        else: selected.add(pid)
                        changed += 1
                self.console.ok(f"Toggled {changed} package(s).")
            except Exception:
                self.console.warn("Unknown command. Try: 1,3,5 | all | none | search vscode | add <id> | go")