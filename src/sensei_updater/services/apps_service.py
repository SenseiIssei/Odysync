# sensei_updater/services/apps_service.py
import re, json, time, shutil, os, hashlib, platform
from typing import List, Dict, Any, Optional, Callable
from pathlib import Path
from ..core.process import Process
from ..core.console import Console
from ..data.paths import CONFIG_DIR

def looks_like_version(s: str) -> bool:
    return bool(re.fullmatch(r"[0-9]+(\.[0-9A-Za-z\-+]+)+", s or ""))

def looks_like_id(s: str) -> bool:
    return bool(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9\-\._]+[A-Za-z0-9]", s or "")) and (" " not in s) and ("." in s)

class AppService:
    def __init__(self, console: Console, cfg):
        self.console = console
        self.proc = Process(debug=console.debug, dry_run=console.dry_run)
        self.cfg = cfg
        self.cache_path = CONFIG_DIR / "last-upgrades.json"
        self.cache_ttl_min = int((self.cfg.get_defaults() or {}).get("cache_ttl_minutes", 10))
        self._silent_cache: Dict[str, bool] = {}
        self.rules_path = CONFIG_DIR / "web_version_rules.json"

    def _has(self, exe: str) -> bool:
        return shutil.which(exe) is not None

    def _cache_write(self, rows: List[Dict[str, Any]]) -> None:
        try:
            self.cache_path.write_text(json.dumps({"ts": time.time(), "rows": rows}, indent=2), encoding="utf-8")
        except Exception:
            pass

    def _cache_read(self) -> List[Dict[str, Any]]:
        try:
            if self.cache_path.exists():
                j = json.loads(self.cache_path.read_text(encoding="utf-8"))
                ts = float(j.get("ts") or 0)
                if ts and (time.time() - ts) <= self.cache_ttl_min * 60:
                    rows = j.get("rows") or []
                    if isinstance(rows, list):
                        return rows
        except Exception:
            pass
        return []

    def _winget_json(self, args: List[str]) -> Any:
        base = ["winget"] + args + ["--accept-source-agreements","--accept-package-agreements","--disable-interactivity","--output","json"]
        rc, out, to = self.proc.run_capture_timeout(base, 60)
        if to or rc != 0 or not out:
            return []
        s = (out or "").strip()
        if not s or (not s.startswith("{") and not s.startswith("[")):
            return []
        try:
            return json.loads(s)
        except Exception:
            return []

    def _parse_table(self, text: str) -> List[Dict[str, Any]]:
        lines = [ln.rstrip() for ln in (text or "").splitlines()]
        rows: List[Dict[str, Any]] = []
        for s in lines:
            t = s.strip()
            if not t:
                continue
            low = t.lower()
            if low.startswith("name ") or low.startswith("id ") or low.startswith("found ") or low.startswith("the following") or low.startswith("keine ") or low.startswith("no "):
                continue
            cols = re.split(r"\s{2,}", t)
            if len(cols) < 2:
                continue
            name = cols[0]
            rest = cols[1:]
            pid, versions, source = "", [], ""
            for c in rest:
                if not pid and looks_like_id(c):
                    pid = c
                    continue
                if looks_like_version(c):
                    versions.append(c)
                    continue
                cl = c.lower()
                if not source and cl in ("winget","msstore","store"):
                    source = c
            installed = versions[0] if len(versions) >= 1 else ""
            available = versions[1] if len(versions) >= 2 else ""
            if not pid:
                continue
            rows.append({"Name": name, "Id": pid, "Version": installed, "Available": available, "Source": source or "winget"})
        dedup: Dict[str, Dict[str, Any]] = {}
        for r in rows:
            dedup[r["Id"]] = r
        return list(dedup.values())

    def _supports_silent(self, pid: str) -> bool:
        if pid in self._silent_cache:
            return self._silent_cache[pid]
        info = self._winget_json(["show","--id",pid,"--source","winget"])
        ok = False
        try:
            items = [info] if isinstance(info, dict) else (info if isinstance(info, list) else [])
            installers = []
            for it in items:
                installers += (it.get("Installers") or it.get("InstallersList") or [])
            for inst in installers:
                switches = inst.get("InstallerSwitches") or {}
                if switches.get("Silent") or switches.get("SilentWithProgress"):
                    ok = True
                    break
                itype = (inst.get("InstallerType") or "").lower()
                if itype in ("msix","appx"):
                    ok = True
                    break
        except Exception:
            ok = False
        self._silent_cache[pid] = ok
        return ok

    def _filter_updatable(self, rows: List[Dict[str, Any]], skip_store: bool=True) -> List[Dict[str, Any]]:
        filtered: List[Dict[str, Any]] = []
        for r in rows:
            pid = r.get("Id","")
            if not looks_like_id(pid):
                continue
            src = (r.get("Source") or "").lower()
            if skip_store and src in ("msstore","store"):
                continue
            available = r.get("Available") or ""
            installed = r.get("Version") or ""
            if not available or available == installed:
                continue
            if src in ("","winget") and not self._supports_silent(pid):
                continue
            filtered.append(r)
        return filtered

    def _run_winget_sources(self) -> None:
        if not self._has("winget"):
            return
        self.proc.run_capture_timeout([
            "winget","source","update","--disable-interactivity"
        ], 20)

    def _list_winget_fast(self) -> List[Dict[str, Any]]:
        if not self._has("winget"):
            return []
        data = self._winget_json(["upgrade","--include-unknown","--source","winget"])
        rows: List[Dict[str, Any]] = []
        arr = data.get("InstalledPackages", []) if isinstance(data, dict) else (data if isinstance(data, list) else [])
        for p in arr or []:
            pid = p.get("PackageIdentifier") or p.get("Id") or ""
            if not looks_like_id(pid):
                continue
            rows.append({
                "Id": pid,
                "Name": p.get("PackageName") or p.get("Name") or pid,
                "Version": p.get("InstalledVersion") or p.get("Version") or "",
                "Available": p.get("AvailableVersion") or p.get("Available") or "",
                "Source": (p.get("Source") or "winget"),
                "Interactive": p.get("IsInteractive", False),
            })
        if rows:
            return rows
        data2 = self._winget_json(["upgrade","--include-unknown"])
        arr2 = data2.get("InstalledPackages", []) if isinstance(data2, dict) else (data2 if isinstance(data2, list) else [])
        for p in arr2 or []:
            pid = p.get("PackageIdentifier") or p.get("Id") or ""
            if not looks_like_id(pid):
                continue
            rows.append({
                "Id": pid,
                "Name": p.get("PackageName") or p.get("Name") or pid,
                "Version": p.get("InstalledVersion") or p.get("Version") or "",
                "Available": p.get("AvailableVersion") or p.get("Available") or "",
                "Source": (p.get("Source") or "winget"),
                "Interactive": p.get("IsInteractive", False),
            })
        if rows:
            return rows
        rc, out, to = self.proc.run_capture_timeout([
            "winget","upgrade","--source","winget","--include-unknown",
            "--accept-source-agreements","--accept-package-agreements","--disable-interactivity"
        ], 25)
        if (rc == 0) and (not to) and out:
            return self._parse_table(out)
        return []

    def _list_choco(self) -> List[Dict[str, Any]]:
        if not self._has("choco"):
            return []
        rc, out = self.proc.run_capture(["choco","outdated","-r"])
        if rc not in (0, 2):
            out = out or ""
        rows = []
        for line in (out or "").splitlines():
            parts = [x.strip() for x in line.split("|")]
            if len(parts) >= 3 and looks_like_id(parts[0]):
                rows.append({"Id": parts[0], "Name": parts[0], "Version": parts[1], "Available": parts[2], "Source": "choco"})
        return rows

    def _list_scoop(self) -> List[Dict[str, Any]]:
        if not self._has("scoop"):
            return []
        rc, out = self.proc.run_capture(["scoop","status","-r"])
        rows = []
        for line in (out or "").splitlines():
            t = line.strip()
            if not t or "is up to date" in t.lower():
                continue
            if "->" in t:
                name = t.split()[0]
                ver = t.split("->")[-1].strip()
                if name:
                    rows.append({"Id": name, "Name": name, "Version": "", "Available": ver, "Source": "scoop"})
        return rows

    def _hash_id(self, name: str) -> str:
        return hashlib.sha256(name.encode("utf-8")).hexdigest()[:12]

    def _enum_registry(self, on_progress: Optional[Callable[[int, str, str], None]] = None) -> List[Dict[str, Any]]:
        if platform.system() != "Windows":
            return []
        import winreg
        hives = [
            (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
            (winreg.HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
            (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        ]
        total = len(hives)
        idx = 0
        rows: List[Dict[str, Any]] = []
        for hive, path in hives:
            idx += 1
            if on_progress:
                on_progress(min(10 + int(idx/total*10), 20), "Scanning registry", path)
            try:
                with winreg.OpenKey(hive, path) as k:
                    i = 0
                    while True:
                        try:
                            sub = winreg.EnumKey(k, i)
                        except OSError:
                            break
                        i += 1
                        try:
                            with winreg.OpenKey(k, sub) as sk:
                                name = version = publisher = installloc = ""
                                try: name = winreg.QueryValueEx(sk, "DisplayName")[0]
                                except Exception: pass
                                try: version = winreg.QueryValueEx(sk, "DisplayVersion")[0]
                                except Exception: pass
                                try: publisher = winreg.QueryValueEx(sk, "Publisher")[0]
                                except Exception: pass
                                try: installloc = winreg.QueryValueEx(sk, "InstallLocation")[0]
                                except Exception: pass
                                if name and version:
                                    rows.append({
                                        "Id": f"reg:{self._hash_id(name)}",
                                        "Name": name,
                                        "Version": version,
                                        "Available": "",
                                        "Source": "registry",
                                        "Publisher": publisher or "",
                                        "InstallLocation": installloc or ""
                                    })
                        except Exception:
                            pass
            except Exception:
                pass
        return rows

    def _list_store(self) -> List[Dict[str, Any]]:
        if platform.system() != "Windows":
            return []
        rc, out = self.proc.run_capture(
            ["powershell.exe","-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command",
             "$p=Get-AppxPackage | Select Name,Version | ConvertTo-Json -Depth 3"]
        )
        if rc != 0 or not out:
            return []
        try:
            data = json.loads(out)
        except Exception:
            data = []
        arr = data if isinstance(data, list) else ([data] if isinstance(data, dict) else [])
        rows: List[Dict[str, Any]] = []
        for d in arr:
            nm = d.get("Name") or ""
            ver = str(d.get("Version") or "")
            if nm and ver:
                rows.append({
                    "Id": f"store:{nm}",
                    "Name": nm,
                    "Version": ver,
                    "Available": "",
                    "Source": "msstore"
                })
        return rows

    def _load_rules(self) -> List[Dict[str, str]]:
        try:
            if self.rules_path.exists():
                rules = json.loads(self.rules_path.read_text(encoding="utf-8"))
                if isinstance(rules, list):
                    norm = []
                    for r in rules:
                        m = (r.get("match") or "").strip()
                        u = (r.get("url") or "").strip()
                        rg = (r.get("regex") or "").strip()
                        if m and u and rg:
                            norm.append({"match": m, "url": u, "regex": rg})
                    return norm
        except Exception:
            return []
        return []

    def _web_latest_for(self, name: str) -> Optional[str]:
        rules = self._load_rules()
        for r in rules:
            if r["match"].lower() in name.lower():
                ps = f"$r=Invoke-WebRequest -UseBasicParsing -Uri '{r['url']}'; $r.Content"
                rc, out = self.proc.run_capture(
                    ["powershell.exe","-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command", ps]
                )
                if rc != 0 or not out:
                    continue
                m = re.search(r[r["regex"]], out)
                if m:
                    v = m.group(1) if m.groups() else m.group(0)
                    if looks_like_version(v):
                        return v
        return None

    def _augment_with_web(self, installed_rows: List[Dict[str, Any]], base_rows: List[Dict[str, Any]], skip_store: bool, on_progress: Optional[Callable[[int, str, str], None]]) -> List[Dict[str, Any]]:
        if on_progress:
            on_progress(85, "Checking vendor pages", "Version rules")
        known_ids = {(r.get("Id") or "", (r.get("Source") or "").lower()) for r in base_rows}
        out_rows = list(base_rows)
        count = len(installed_rows) or 1
        for i, r in enumerate(installed_rows):
            if on_progress:
                pct = 85 + int((i / count) * 10)
                on_progress(min(pct, 95), "Checking vendor pages", r.get("Name") or "")
            name = r.get("Name") or ""
            if not name:
                continue
            try:
                web_ver = self._web_latest_for(name)
            except Exception:
                web_ver = None
            if not web_ver:
                continue
            installed = r.get("Version") or ""
            if not installed or installed == web_ver:
                continue
            pid = f"web:{self._hash_id(name)}"
            key = (pid, "web")
            if key in known_ids:
                continue
            if skip_store and (r.get("Source","").lower() in ("msstore","store")):
                continue
            out_rows.append({
                "Id": pid,
                "Name": name,
                "Version": installed,
                "Available": web_ver,
                "Source": "web",
                "Interactive": True
            })
        return out_rows

    def _walk_dirs_progress(self, roots: List[Path], on_progress: Optional[Callable[[int, str, str], None]]):
        total_dirs = 0
        for root in roots:
            for _, dirs, _ in os.walk(root, topdown=True):
                total_dirs += 1
        walked = 0
        for root in roots:
            for dirpath, _, _ in os.walk(root, topdown=True):
                walked += 1
                if on_progress:
                    pct = min(10, int((walked / max(1, total_dirs)) * 10))
                    on_progress(pct, "Scanning files", dirpath)

    def list_upgrades_all(self, force_refresh: bool=False, skip_store: bool=True, on_progress: Optional[Callable[[int, str, str], None]] = None) -> List[Dict[str, Any]]:
        if not force_refresh:
            cached = self._cache_read()
            if cached:
                if on_progress:
                    on_progress(100, "Done", "")
                return cached
        if on_progress:
            on_progress(1, "Preparing", "")
        roots = []
        pf = os.environ.get("ProgramFiles")
        pfx = os.environ.get("ProgramFiles(x86)")
        pd = os.environ.get("ProgramData")
        if pf: roots.append(Path(pf))
        if pfx: roots.append(Path(pfx))
        if pd: roots.append(Path(pd) / "Microsoft" / "Windows" / "Start Menu" / "Programs")
        self._walk_dirs_progress(roots, on_progress)
        if on_progress:
            on_progress(20, "Refreshing sources", "winget")
        self._run_winget_sources()
        if on_progress:
            on_progress(30, "Scanning", "winget")
        winget_rows = self._filter_updatable(self._list_winget_fast(), skip_store=skip_store)
        if on_progress:
            on_progress(40, "Scanning", "choco")
        choco_rows = self._list_choco()
        if on_progress:
            on_progress(45, "Scanning", "scoop")
        scoop_rows = self._list_scoop()
        reg_rows = self._enum_registry(on_progress)
        if on_progress:
            on_progress(55, "Scanning", "msstore")
        store_rows = self._list_store()
        if on_progress:
            on_progress(70, "Merging results", "")
        base = winget_rows + choco_rows + scoop_rows
        installed_union = reg_rows + store_rows
        merged = self._augment_with_web(installed_union, base, skip_store=skip_store, on_progress=on_progress)
        if on_progress:
            on_progress(96, "Deduplicating", "")
        seen = set()
        final = []
        for r in merged:
            key = (r.get("Id") or "", r.get("Source") or "")
            if key in seen:
                continue
            seen.add(key)
            final.append(r)
        if final:
            self._cache_write(final)
        if on_progress:
            on_progress(100, "Done", "")
        return final

    def update_ids(self, ids: List[str]) -> Dict[str, List[str]]:
        results = {"updated": [], "interactive": [], "reinstalled": [], "skipped": [], "store_skipped": [], "failed": []}
        for pid in ids:
            if pid.startswith("web:"):
                results["skipped"].append(pid)
                continue
            if not looks_like_id(pid) or looks_like_version(pid):
                results["skipped"].append(pid)
                continue
            ok = False
            if self._has("winget"):
                rc, to = self.proc.run_stream_timeout(["winget","upgrade","--id",pid,"--silent","--disable-interactivity","--accept-source-agreements","--accept-package-agreements","--force"], 900)
                ok = (rc in (0,3010)) and (not to)
                if not ok:
                    rc2, to2 = self.proc.run_stream_timeout(["winget","install","--id",pid,"--silent","--disable-interactivity","--accept-source-agreements","--accept-package-agreements","--force"], 900)
                    ok = (rc2 in (0,3010)) and (not to2)
            if not ok and self._has("choco"):
                rc, to = self.proc.run_stream_timeout(["choco","upgrade",pid,"-y"], 600)
                ok = (rc == 0) and (not to)
            if not ok and self._has("scoop"):
                rc, to = self.proc.run_stream_timeout(["scoop","update",pid], 600)
                ok = (rc == 0) and (not to)
            (results["updated"] if ok else results["failed"]).append(pid)
        return results