import json
from ..data.paths import CONFIG_DIR, CONFIG_PATH, SETTINGS_PATH

class ConfigStore:
    def __init__(self):
        CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        self.data = self._load_json(CONFIG_PATH) or {}
        self.settings = self._load_json(SETTINGS_PATH) or {
            "schedule": {
                "enabled": False,
                "frequency": None,
                "time": "09:00",
                "task_name": "SenseisUpdater Auto Update",
                "args": ["--apps","--yes","--report","json","--out","%LOCALAPPDATA%\\SenseiUpdater\\last-run.json"]
            },
            "defaults": {
                "profile": None,
                "yes": False,
                "report": "json",
                "out": "%LOCALAPPDATA%\\SenseiUpdater\\last-run.json",
                "prefer_tui": False,
                "cache_ttl_minutes": 15
            }
        }

    def _load_json(self, path):
        if path.exists():
            try:
                return json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                return None
        return None

    def _atomic_write(self, path, payload):
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        tmp.replace(path)

    def save(self):
        self._atomic_write(CONFIG_PATH, self.data)

    def save_settings(self):
        self._atomic_write(SETTINGS_PATH, self.settings)

    def list_profiles(self):
        return sorted(self.data.get("profiles", {}).keys())

    def get_profile(self, name: str):
        return set(self.data.get("profiles", {}).get(name, []))

    def set_profile(self, name: str, ids):
        self.data.setdefault("profiles", {})[name] = sorted(set(ids))
        self.save()

    def get_schedule(self):
        s = self.settings.get("schedule") or {}
        s.setdefault("enabled", False)
        s.setdefault("frequency", None)
        s.setdefault("time", "09:00")
        s.setdefault("task_name", "SenseisUpdater Auto Update")
        s.setdefault("args", ["--apps","--yes","--report","json","--out","%LOCALAPPDATA%\\SenseiUpdater\\last-run.json"])
        self.settings["schedule"] = s
        return s

    def set_schedule(self, enabled: bool, frequency: str | None, time_str: str, task_name: str, args: list[str]):
        self.settings["schedule"] = {"enabled": bool(enabled), "frequency": frequency, "time": time_str, "task_name": task_name, "args": list(args)}
        self.save_settings()

    def get_defaults(self):
        d = self.settings.get("defaults") or {}
        d.setdefault("profile", None)
        d.setdefault("yes", False)
        d.setdefault("report", "json")
        d.setdefault("out", "%LOCALAPPDATA%\\SenseiUpdater\\last-run.json")
        d.setdefault("prefer_tui", False)
        d.setdefault("cache_ttl_minutes", 15)
        self.settings["defaults"] = d
        return d

    def set_defaults(self, kv: dict):
        d = self.get_defaults()
        d.update(kv or {})
        self.settings["defaults"] = d
        self.save_settings()

    def export_profiles(self, path: str):
        payload = {"profiles": self.data.get("profiles", {})}
        from pathlib import Path
        p = Path(path).expanduser()
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        return str(p)

    def import_profiles(self, path: str, merge: bool = True):
        from pathlib import Path
        p = Path(path).expanduser()
        if not p.exists():
            return False, "file_not_found"
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
            incoming = data.get("profiles", {})
            if not isinstance(incoming, dict):
                return False, "invalid_format"
            cur = self.data.get("profiles", {}) if merge else {}
            for k, v in incoming.items():
                cur[k] = sorted(set(v or []))
            self.data["profiles"] = cur
            self.save()
            return True, None
        except Exception:
            return False, "read_error"