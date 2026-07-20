from ..core.colors import *
from .selector import Selector
from ..domain.reports import RunReport

class Menu:
    def __init__(self, console, app, drivers, system, cfg, scheduler=None):
        self.console = console
        self.app = app
        self.drivers = drivers
        self.system = system
        self.cfg = cfg
        self.selector = Selector(console, cfg)
        self.scheduler = scheduler

    def _print_summary(self, r: RunReport):
        r.mark_finished()
        self.console.header("Summary")
        print(f"Driver success: {r.driver_success}")
        print(f"Reboot required: {r.reboot_required}")
        def show(label, arr, color=WHITE):
            if arr:
                print(f"{color}{label}:{RESET} " + ", ".join(arr))
        show("Updated", r.updated, GREEN)
        show("Updated (interactive)", r.interactive, CYAN)
        show("Reinstalled", r.reinstalled, MAGENTA)
        show("Skipped", r.skipped, GRAY)
        show("Store skipped (admin)", r.store_skipped, YELLOW)
        show("Failed", r.failed, RED)
        if r.notes:
            print("Notes: " + "; ".join(r.notes))

    def _schedule_menu(self):
        s = self.cfg.get_schedule()
        while True:
            self.console.header("Scheduling & Auto-Update")
            print(f"Enabled: {GREEN}Yes{RESET}" if s.get("enabled") else f"Enabled: {RED}No{RESET}")
            print(f"Frequency: {s.get('frequency') or '-'}")
            print(f"Time: {s.get('time')}")
            print(f"Task name: {s.get('task_name')}")
            print(f"Args: {' '.join(s.get('args') or [])}")
            print()
            print("1) Enable weekly")
            print("2) Enable monthly")
            print("3) Disable scheduling")
            print("4) Change time (HH:MM)")
            print("5) Change task name")
            print("6) Edit args")
            print("7) Save and apply")
            print("0) Back")
            choice = input(f"{ORANGE2}{BOLD}Select → {RESET}").strip()
            if choice == "1":
                s["enabled"] = True; s["frequency"] = "weekly"
            elif choice == "2":
                s["enabled"] = True; s["frequency"] = "monthly"
            elif choice == "3":
                s["enabled"] = False
            elif choice == "4":
                t = input("Time HH:MM → ").strip()
                if t: s["time"] = t
            elif choice == "5":
                n = input("Task name → ").strip()
                if n: s["task_name"] = n
            elif choice == "6":
                a = input("Args line → ").strip()
                if a:
                    s["args"] = a.split()
            elif choice == "7":
                self.cfg.set_schedule(s.get("enabled"), s.get("frequency"), s.get("time"), s.get("task_name"), s.get("args"))
                if self.scheduler:
                    if s.get("enabled") and s.get("frequency"):
                        if self.scheduler.exists(s["task_name"]):
                            self.scheduler.delete(s["task_name"])
                        ok = self.scheduler.create(s["frequency"], s["time"], s["task_name"], s["args"])
                        if ok: self.console.ok("Schedule applied.")
                        else: self.console.err("Failed to apply schedule.")
                    else:
                        if self.scheduler.exists(s["task_name"]):
                            if self.scheduler.delete(s["task_name"]):
                                self.console.ok("Schedule removed.")
                else:
                    self.console.warn("Scheduler unavailable.")
            elif choice == "0":
                return
            else:
                self.console.warn("Invalid choice.")

    def _defaults_menu(self):
        d = self.cfg.get_defaults()
        while True:
            self.console.header("Defaults")
            print(f"Default profile: {d.get('profile') or '-'}")
            print(f"Default yes: {d.get('yes')}")
            print(f"Default report: {d.get('report')}")
            print(f"Default out: {d.get('out')}")
            print(f"Prefer TUI: {d.get('prefer_tui')}")
            print()
            print("1) Set default profile")
            print("2) Toggle default yes")
            print("3) Set default report (json|txt)")
            print("4) Set default out path")
            print("5) Toggle prefer TUI")
            print("6) Save")
            print("0) Back")
            choice = input(f"{ORANGE2}{BOLD}Select → {RESET}").strip()
            if choice == "1":
                p = input("Profile name → ").strip()
                d["profile"] = p or None
            elif choice == "2":
                d["yes"] = not bool(d.get("yes"))
            elif choice == "3":
                r = input("json or txt → ").strip().lower()
                if r in ("json","txt"):
                    d["report"] = r
            elif choice == "4":
                o = input("Output path → ").strip()
                if o:
                    d["out"] = o
            elif choice == "5":
                d["prefer_tui"] = not bool(d.get("prefer_tui"))
            elif choice == "6":
                self.cfg.set_defaults(d)
                self.console.ok("Defaults saved.")
            elif choice == "0":
                return
            else:
                self.console.warn("Invalid choice.")

    def _profiles_menu(self):
        while True:
            self.console.header("Profiles")
            names = self.cfg.list_profiles()
            if names:
                print("Profiles: " + ", ".join(names))
            else:
                print("No profiles saved.")
            print()
            print("1) Create or overwrite profile from current upgrade list")
            print("2) Add package Id to profile")
            print("3) Remove package Id from profile")
            print("4) Export profiles to JSON")
            print("5) Import profiles from JSON")
            print("0) Back")
            choice = input(f"{ORANGE2}{BOLD}Select → {RESET}").strip()
            if choice == "1":
                pname = input("Profile name → ").strip()
                if not pname:
                    continue
                rows = self.app.list_upgrades()
                ids = [r.get("Id") for r in (rows or []) if r.get("Id")]
                if not ids:
                    self.console.warn("No upgradeable apps found to store.")
                self.cfg.set_profile(pname, ids)
                self.console.ok(f"Saved {len(ids)} ids to profile '{pname}'.")
            elif choice == "2":
                pname = input("Profile name → ").strip()
                pid = input("Package Id → ").strip()
                cur = set(self.cfg.get_profile(pname))
                if pid:
                    cur.add(pid)
                    self.cfg.set_profile(pname, sorted(cur))
                    self.console.ok(f"Added {pid} to '{pname}'.")
            elif choice == "3":
                pname = input("Profile name → ").strip()
                pid = input("Package Id → ").strip()
                cur = set(self.cfg.get_profile(pname))
                if pid in cur:
                    cur.remove(pid)
                    self.cfg.set_profile(pname, sorted(cur))
                    self.console.ok(f"Removed {pid} from '{pname}'.")
            elif choice == "4":
                path = input("Export path → ").strip()
                if path:
                    out = self.cfg.export_profiles(path)
                    self.console.ok(f"Exported to {out}")
            elif choice == "5":
                path = input("Import path → ").strip()
                if path:
                    ok, err = self.cfg.import_profiles(path, merge=True)
                    if ok:
                        self.console.ok("Imported profiles.")
                    else:
                        self.console.err(f"Import failed: {err}")
            elif choice == "0":
                return
            else:
                self.console.warn("Invalid choice.")

    def run(self):
        while True:
            self.console.pixel_art()
            ctx = "Administrator" if self.console.is_admin() else "User"
            print(f"{GRAY}Context: {ctx}  •  Run app updates as User; run drivers/cleanup/health as Admin.{RESET}")
            self.console.header("Sensei's Updater — Menu")
            print(f"{ORANGE1}{BOLD}1){RESET} Create Restore Point  {GRAY}(Admin){RESET}")
            print(f"{ORANGE1}{BOLD}2){RESET} Update Drivers (Windows Update — Drivers)  {GRAY}(Admin){RESET}")
            print(f"{ORANGE1}{BOLD}3){RESET} Update Applications (choose dynamically)  {GRAY}(User recommended){RESET}")
            print(f"{ORANGE1}{BOLD}4){RESET} Cleanup TEMP + Empty Recycle Bin  {GRAY}(Admin){RESET}")
            print(f"{ORANGE1}{BOLD}5){RESET} Health Scan (DISM + SFC)  {GRAY}(Admin){RESET}")
            print(f"{ORANGE1}{BOLD}6){RESET} Show Startup Programs")
            print(f"{ORANGE1}{BOLD}7){RESET} QUICK: 1+2+3+4 in one go  {GRAY}(Admin){RESET}")
            print(f"{ORANGE1}{BOLD}8){RESET} Scheduling & Auto-Update")
            print(f"{ORANGE1}{BOLD}9){RESET} Defaults")
            print(f"{ORANGE1}{BOLD}10){RESET} Profiles")
            print(f"{ORANGE1}{BOLD}11){RESET} Open Microsoft Store Library")
            print(f"{RED}{BOLD}0){RESET} Exit")
            print(f"{DIM}{GRAY}Support: https://ko-fi.com/senseiissei{RESET}")
            choice = input(f"{ORANGE2}{BOLD}Select → {RESET}").strip()

            if choice == "1":
                self.drivers.create_restore_point()
            elif choice == "2":
                report = RunReport()
                ok, reboot = self.drivers.update_drivers()
                report.driver_success = ok
                report.reboot_required = reboot
                self._print_summary(report)
            elif choice == "3":
                upgrades = self.app.list_upgrades()
                if upgrades:
                    chosen = self.selector.loop(upgrades, self.app, title="Upgradable Apps (winget)")
                else:
                    self.console.warn("No upgrades detected by winget.")
                    self.console.info("Showing installed apps so you can pick targets by Id.")
                    installed = self.app.list_installed()
                    if not installed:
                        self.console.warn("Could not read installed apps. Try updating winget (App Installer).")
                        continue
                    chosen = self.selector.loop(installed, self.app, title="Installed Apps (select any to attempt upgrade)")
                if chosen:
                    self.console.header("Update Plan (per-package)")
                    for pid in chosen: print(f"{GREEN}✔ {pid}{RESET}")
                    print()
                    confirm = input(f"{ORANGE1}{BOLD}Proceed with these updates? (y/N) {RESET}").strip().lower()
                    if confirm == "y":
                        r = RunReport()
                        res = self.app.update_ids(chosen)
                        r.updated.extend(res["updated"])
                        r.interactive.extend(res["interactive"])
                        r.reinstalled.extend(res["reinstalled"])
                        r.skipped.extend(res["skipped"])
                        r.store_skipped.extend(res["store_skipped"])
                        r.failed.extend(res["failed"])
                        self._print_summary(r)
            elif choice == "4":
                self.system.cleanup_temp(); self.system.empty_recycle_bin()
            elif choice == "5":
                self.system.dism_sfc()
            elif choice == "6":
                self.system.show_startup()
            elif choice == "7":
                r = RunReport()
                self.drivers.create_restore_point("Sensei_Quick_RP")
                ok, reboot = self.drivers.update_drivers()
                r.driver_success = ok
                r.reboot_required = reboot
                upgrades = self.app.list_upgrades()
                if upgrades:
                    res = self.app.update_ids([p["Id"] for p in upgrades])
                    r.updated.extend(res["updated"])
                    r.interactive.extend(res["interactive"])
                    r.reinstalled.extend(res["reinstalled"])
                    r.skipped.extend(res["skipped"])
                    r.store_skipped.extend(res["store_skipped"])
                    r.failed.extend(res["failed"])
                self.system.cleanup_temp(); self.system.empty_recycle_bin()
                self.console.header("Quick Maintenance")
                self.console.ok("All quick tasks completed. If drivers were installed, consider rebooting.")
                self._print_summary(r)
            elif choice == "8":
                self._schedule_menu()
            elif choice == "9":
                self._defaults_menu()
            elif choice == "10":
                self._profiles_menu()
            elif choice == "11":
                if self.system.open_store_library():
                    self.console.ok("Opened Microsoft Store → Library.")
                else:
                    self.console.err("Failed to open Microsoft Store Library.")
            elif choice == "0":
                print(f"{RED}{BOLD}Bye!{RESET}")
                break
            else:
                self.console.warn("Invalid choice. Try again.")