import os, ctypes

def is_admin() -> bool:
    if os.name != "nt": return False
    try: return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception: return False

def require_admin_or_msg(console, name: str) -> bool:
    if is_admin(): return True
    console.err(f"{name} requires Administrator.")
    console.info("Re-open Terminal/PowerShell as Administrator and try again.")
    return False