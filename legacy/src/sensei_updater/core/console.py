import os, ctypes, sys, io
from datetime import datetime
from .colors import *
from ..data.paths import KO_FI_URL, LOG_DIR

class Console:
    def __init__(self, debug: bool=False, dry_run: bool=False):
        self.debug = debug
        self.dry_run = dry_run
        self._log_fp = None

    def enable_windows_ansi_utf8(self):
        if os.name == "nt":
            try:
                k32 = ctypes.windll.kernel32
                hOut = k32.GetStdHandle(-11)
                mode = ctypes.c_uint32()
                if k32.GetConsoleMode(hOut, ctypes.byref(mode)):
                    k32.SetConsoleMode(hOut, mode.value | 0x0004)
                k32.SetConsoleOutputCP(65001)
                k32.SetConsoleCP(65001)
            except Exception:
                pass
        try:
            LOG_DIR.mkdir(parents=True, exist_ok=True)
            log_path = LOG_DIR / f"run-{datetime.now().strftime('%Y%m%d-%H%M%S')}.log"
            self._log_fp = open(log_path, "w", encoding="utf-8", errors="replace")
            class Tee(io.TextIOBase):
                def __init__(self, a, b): self.a, self.b = a, b
                def write(self, s): self.a.write(s); self.b.write(s); return len(s)
                def flush(self): self.a.flush(); self.b.flush()
            sys.stdout = Tee(sys.stdout, self._log_fp)
            sys.stderr = Tee(sys.stderr, self._log_fp)
        except Exception:
            pass

    def close(self):
        try:
            if self._log_fp: self._log_fp.close()
        except Exception:
            pass

    def is_admin(self) -> bool:
        if os.name != "nt": return False
        try:
            return bool(ctypes.windll.shell32.IsUserAnAdmin())
        except Exception:
            return False

    def header(self, title: str):
        print(f"{ORANGE2}{BOLD}{'='*80}{RESET}")
        print(f"{ORANGE2}{BOLD}{title}{RESET}")
        print(f"{ORANGE2}{BOLD}{'='*80}{RESET}")

    def info(self, msg): print(f"{CYAN}→ {msg}{RESET}")
    def ok(self, msg):   print(f"{GREEN}✔ {msg}{RESET}")
    def warn(self, msg): print(f"{YELLOW}⚠ {msg}{RESET}")
    def err(self, msg):  print(f"{RED}{BOLD}✘ {msg}{RESET}")

    def banner(self):
        ctx = "Administrator" if self.is_admin() else "User"
        ctx_color = RED if self.is_admin() else GREEN
        print(f"\n{SUN}{BOLD}Sensei's Updater{RESET}  {GRAY}— clean • update • repair (safely){RESET}")
        print(f"{ctx_color}{BOLD}Context:{RESET} {ctx}")
        print(f"{GRAY}If you enjoy the updater, you can support me: {WHITE}{BOLD}{KO_FI_URL}{RESET}\n")

    def pixel_art(self):
        art = [
            f"{GRAY}{DIM}                 ..:::{RESET}{BROWN}▀▀▀▀▀▀{RESET}{GRAY}{DIM}:::..                 {RESET}",
            f"{GRAY}{DIM}             ..:::{RESET}{BROWN}▀▀▀{RESET}{SUN}{BOLD}▀▀{RESET}{BROWN}▀▀▀{RESET}{GRAY}{DIM}:::..             {RESET}",
            f"           {BROWN}▄{SUN}████████{BROWN}▄{SUN}████████{BROWN}▄{RESET}",
            f"         {BROWN}▄{SUN}██████████████████████{BROWN}▄{RESET}",
            f"       {BROWN}▄{SUN}████{BROWN}▄{SUN}████████████████{BROWN}▄{SUN}████{BROWN}▄{RESET}",
            f"      {BROWN}█{SUN}██{BROWN}█{SUN}██{BROWN}█  {RESET}{WHITE}{BOLD}●{RESET}{SUN}██████{WHITE}{BOLD}●{RESET}{SUN}  {BROWN}█{SUN}██{BROWN}█{SUN}██{BROWN}█{RESET}",
            f"      {BROWN}█{SUN}██{BROWN}█{SUN}██{BROWN}█ {RESET}{ORANGE1}{BOLD}●{RESET}{SUN}████████{ORANGE1}{BOLD}●{RESET}{SUN} {BROWN}█{SUN}██{BROWN}█{SUN}██{BROWN}█{RESET}",
            f"      {BROWN}█{SUN}████████  {RED}{BOLD}▂▂{RESET}{SUN}  {RED}{BOLD}▂▂{RESET}{SUN}  ████{BROWN}█{RESET}",
            f"       {BROWN}▀{SUN}██████████████████████{BROWN}▀{RESET}",
            f"          {BROWN}▄{SUN}██{BROWN}▄       {SUN}██       {BROWN}▄{SUN}██{BROWN}▄{RESET}",
            f"         {BROWN}█{SUN}███████{AMBER}██████████{SUN}███████{BROWN}█{RESET}",
            f"        {BROWN}█{SUN}██{BROWN}▄  {SUN}███{BROWN}▄      ▄{SUN}███  {BROWN}▄{SUN}██{BROWN}█{RESET}",
            f"        {BROWN}▀{SUN}██{BROWN}▀   {SUN}██{BROWN}▀      ▀{SUN}██   {BROWN}▀{SUN}██{BROWN}▀{RESET}",
            ""
        ]
        print("\n".join(art))