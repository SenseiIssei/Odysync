import argparse, os
from pathlib import Path
from .core.console import Console
from .domain.config import ConfigStore
from .domain.reports import RunReport
from .services.drivers import DriverService
from .services.apps import AppService
from .services.system import SystemService
from .services.diagnostics import DiagnosticsService
from .services.scheduler import SchedulerService
from .ui.menu import Menu

def main():
    parser = argparse.ArgumentParser(description="Sensei's Updater")
    parser.add_argument("--quick", action="store_true")
    parser.add_argument("--drivers", action="store_true")
    parser.add_argument("--apps", action="store_true")
    parser.add_argument("--cleanup", action="store_true")
    parser.add_argument("--health", action="store_true")
    parser.add_argument("--startup", action="store_true")
    parser.add_argument("--profile", type=str, default=None)
    parser.add_argument("--yes", action="store_true")
    parser.add_argument("--report", choices=["json","txt"])
    parser.add_argument("--out", type=str)
    parser.add_argument("--diagnostics", action="store_true")
    parser.add_argument("--diag-out", type=str)
    parser.add_argument("--schedule", choices=["weekly","monthly"])
    parser.add_argument("--time", type=str, default="09:00")
    parser.add_argument("--task-name", type=str, default="SenseisUpdater Auto Update")
    parser.add_argument("--unschedule", action="store_true")
    parser.add_argument("--tui", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--debug", action="store_true")
    parser.add_argument("--export", type=str)
    parser.add_argument("--import", dest="import_path", type=str)

    args = parser.parse_args()

    console = Console(debug=args.debug, dry_run=args.dry_run)
    console.enable_windows_ansi_utf8()
    console.banner()

    cfg = ConfigStore()
    defaults = cfg.get_defaults()
    if args.profile is None and defaults.get("profile"):
        args.profile = defaults.get("profile")
    if not args.yes and defaults.get("yes"):
        args.yes = True
    if not args.report:
        args.report = defaults.get("report") or "json"
    if not args.out:
        args.out = defaults.get("out")
    prefer_tui = bool(defaults.get("prefer_tui"))

    app = AppService(console=console, cfg=cfg)
    drivers = DriverService(console=console)
    system = SystemService(console=console)
    diag = DiagnosticsService(console=console, cfg=cfg, app=app, system=system)
    sched = SchedulerService(console=console)

    app.check_environment()
    try:
        if system.has_pending_reboot():
            console.warn("Windows indicates a pending reboot. Consider rebooting before updates to avoid conflicts.")
    except Exception:
        pass

    s = cfg.get_schedule()
    if s.get("enabled") and s.get("frequency") and s.get("task_name"):
        if not sched.exists(s["task_name"]):
            ok = sched.create(s["frequency"], s["time"], s["task_name"], s["args"])
            if ok: console.ok("Applied saved schedule.")
            else: console.warn("Failed to apply saved schedule.")

    if args.unschedule and args.task_name:
        if sched.delete(args.task_name):
            console.ok(f"Removed scheduled task: {args.task_name}")
        else:
            console.warn(f"Could not remove scheduled task: {args.task_name}")
        return

    if args.schedule:
        out_dir = Path(os.getenv("LOCALAPPDATA", str(Path.home()))).joinpath("SenseiUpdater")
        out_dir.mkdir(parents=True, exist_ok=True)
        extra = []
        if args.quick: extra += ["--quick"]
        if args.drivers: extra += ["--drivers"]
        if args.apps: extra += ["--apps"]
        if args.cleanup: extra += ["--cleanup"]
        if args.health: extra += ["--health"]
        if args.startup: extra += ["--startup"]
        if args.profile: extra += ["--profile", args.profile]
        if args.yes: extra += ["--yes"]
        extra += ["--report", args.report or "json", "--out", args.out or str(out_dir.joinpath("last-run.json"))]
        ok = sched.create(args.schedule, args.time, args.task_name, extra)
        if ok:
            cfg.set_schedule(True, args.schedule, args.time, args.task_name, extra)
            console.ok(f"Scheduled {args.schedule} at {args.time}: {args.task_name}")
        else:
            console.err("Scheduling failed.")
        return

    if args.export:
        out_path = cfg.export_profiles(args.export)
        console.ok(f"Exported profiles to {out_path}")
        return

    if args.import_path:
        ok, err = cfg.import_profiles(args.import_path, merge=True)
        if ok:
            console.ok("Imported profiles.")
        else:
            console.err(f"Import failed: {err}")
        return

    if args.tui or prefer_tui:
        try:
            from .ui.tui import run_tui
            run_tui(console, app, cfg)
            return
        except Exception:
            console.err("TUI not available. Install with: pip install '.[tui]'")

    if args.quick or args.drivers or args.apps or args.cleanup or args.health or args.startup or args.profile or args.diagnostics:
        report = RunReport()
        selected_ids = []
        if args.profile:
            selected_ids = list(cfg.get_profile(args.profile))
            if not selected_ids:
                console.warn(f"Profile '{args.profile}' is empty or missing.")

        if args.quick or args.drivers:
            ok, reboot = drivers.update_drivers()
            report.driver_success = ok
            report.reboot_required = report.reboot_required or reboot

        if args.quick or args.apps or args.profile:
            if selected_ids:
                res = app.update_ids(selected_ids)
            else:
                upgrades = app.list_upgrades()
                if upgrades and args.yes:
                    res = app.update_ids([p["Id"] for p in upgrades])
                elif upgrades and not args.yes:
                    from .ui.selector import Selector
                    console.info("Opening interactive selector (no --yes provided).")
                    chosen = Selector(console, cfg).loop(upgrades, app, title="Upgradable Apps (winget)")
                    if chosen:
                        res = app.update_ids(chosen)
                    else:
                        res = {"updated":[], "interactive":[], "reinstalled":[], "skipped":[], "store_skipped":[], "failed":[]}
                else:
                    console.warn("No upgrades detected by winget.")
                    res = {"updated":[], "interactive":[], "reinstalled":[], "skipped":[], "store_skipped":[], "failed":[]}
            report.updated.extend(res["updated"])
            report.interactive.extend(res["interactive"])
            report.reinstalled.extend(res["reinstalled"])
            report.skipped.extend(res["skipped"])
            report.store_skipped.extend(res["store_skipped"])
            report.failed.extend(res["failed"])

        if args.quick or args.cleanup:
            system.cleanup_temp()
            system.empty_recycle_bin()

        if args.quick or args.health:
            system.dism_sfc()

        if args.startup:
            system.show_startup()

        report.mark_finished()
        from .ui.menu import Menu as _Menu
        _Menu(console, app, drivers, system, cfg)._print_summary(report)
        if args.report and args.out:
            out_path = Path(args.out).expanduser()
            try:
                report.save(args.report, out_path)
                console.ok(f"Report written: {out_path}")
            except Exception as e:
                console.warn(f"Could not write report: {e}")
        if args.diagnostics:
            zip_path = Path(args.diag_out).expanduser() if args.diag_out else Path.cwd() / "sensei-diagnostics.zip"
            try:
                diag = DiagnosticsService(console=console, cfg=cfg, app=app, system=system)
                diag.create(zip_path=zip_path, report=report, report_fmt=args.report or "json")
                console.ok(f"Diagnostics zip written: {zip_path}")
            except Exception as e:
                console.warn(f"Could not create diagnostics: {e}")
        return

    from .ui.menu import Menu as _Menu
    menu = _Menu(console=console, app=app, drivers=drivers, system=system, cfg=cfg, scheduler=sched)
    try:
        menu.run()
    finally:
        console.close()