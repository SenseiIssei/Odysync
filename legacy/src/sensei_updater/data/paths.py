from pathlib import Path
import os

APP_NAME = "SenseiUpdater"
KO_FI_URL = "https://ko-fi.com/senseiissei"

BASE_DIR = Path(os.getenv("LOCALAPPDATA", Path.home())) / APP_NAME
CONFIG_DIR = BASE_DIR
CONFIG_PATH = CONFIG_DIR / "config.json"
SETTINGS_PATH = CONFIG_DIR / "settings.json"
LOG_DIR = CONFIG_DIR / "logs"