# core/process.py
import subprocess, sys, time, os, platform
from .colors import MAGENTA, DIM, RESET, GRAY

def _win_creation():
    if platform.system() != "Windows":
        return {}
    si = subprocess.STARTUPINFO()
    si.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    flags = 0x08000000  # CREATE_NO_WINDOW
    return {"startupinfo": si, "creationflags": flags}

class Process:
    def __init__(self, debug: bool=False, dry_run: bool=False):
        self.debug = debug
        self.dry_run = dry_run
        self._win_kwargs = _win_creation()

    def _dbg(self, cmd):
        if self.debug:
            print(f"{MAGENTA}{DIM}>>> {' '.join(map(str,cmd))}{RESET}")

    def run_stream(self, cmd: list[int|str]) -> int:
        self._dbg(cmd)
        if self.dry_run:
            print(f"{GRAY}[dry-run]{RESET} {' '.join(map(str,cmd))}")
            return 0
        try:
            p = subprocess.Popen(
                cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, encoding="utf-8", errors="replace",
                shell=False, **self._win_kwargs
            )
            assert p.stdout is not None
            for line in p.stdout:
                s = (line or "").rstrip("\r\n")
                if s:
                    print(s)
            return p.wait()
        except KeyboardInterrupt:
            try: p.terminate()
            except Exception: pass
            return 1
        except Exception:
            return 1

    def run_capture(self, cmd: list[int|str]) -> tuple[int,str]:
        self._dbg(cmd)
        if self.dry_run:
            return 0, ""
        try:
            r = subprocess.run(
                cmd, capture_output=True, text=True,
                encoding="utf-8", errors="replace",
                shell=False, **self._win_kwargs
            )
            return r.returncode, r.stdout or ""
        except Exception:
            return 1, ""

    def run_capture_timeout(self, cmd: list[int|str], timeout_s: float) -> tuple[int,str,bool]:
        self._dbg(cmd)
        if self.dry_run:
            return 0, "", False
        out = []
        try:
            p = subprocess.Popen(
                cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, encoding="utf-8", errors="replace",
                shell=False, **self._win_kwargs
            )
        except Exception:
            return 1, "", False
        start = time.time()
        try:
            assert p.stdout is not None
            while True:
                if p.poll() is not None:
                    break
                if time.time() - start > timeout_s:
                    try: p.terminate()
                    except Exception: pass
                    try: p.kill()
                    except Exception: pass
                    return 124, "".join(out), True
                chunk = p.stdout.readline()
                if chunk:
                    out.append(chunk)
                else:
                    time.sleep(0.05)
        except Exception:
            try: p.kill()
            except Exception: pass
            return 1, "".join(out), False
        try:
            rc = p.wait(timeout=1)
        except Exception:
            rc = 1
        return rc, "".join(out), False

    def run_stream_progress(self, cmd: list[int|str], label: str, idle_anim: str="|/-\\", idle_tick: float=0.1, idle_after: float=1.0) -> int:
        self._dbg(cmd)
        if self.dry_run:
            print(f"{GRAY}[dry-run]{RESET} {label}")
            return 0
        try:
            p = subprocess.Popen(
                cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, encoding="utf-8", errors="replace",
                shell=False, **self._win_kwargs
            )
            assert p.stdout is not None
            last = time.time()
            frame = 0
            while True:
                line = p.stdout.readline()
                if line:
                    print(line.rstrip("\r\n"))
                    last = time.time()
                else:
                    if p.poll() is not None:
                        break
                    if time.time() - last >= idle_after:
                        ch = idle_anim[frame % len(idle_anim)]
                        sys.stdout.write(f"\r{label} {ch}")
                        sys.stdout.flush()
                        frame += 1
                        time.sleep(idle_tick)
                    else:
                        time.sleep(0.05)
        except Exception:
            try: p.kill()
            except Exception: pass
            return 1
        finally:
            try:
                sys.stdout.write("\r" + " " * (len(label) + 2) + "\r")
                sys.stdout.flush()
            except Exception:
                pass
        try:
            return p.wait(timeout=1)
        except Exception:
            return 1

    def run_stream_timeout(self, cmd: list[int|str], timeout_s: float) -> tuple[int, bool]:
        self._dbg(cmd)
        if self.dry_run:
            return 0, False
        try:
            p = subprocess.Popen(
                cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, encoding="utf-8", errors="replace",
                shell=False, **self._win_kwargs
            )
        except Exception:
            return 1, False
        start = time.time()
        try:
            assert p.stdout is not None
            while True:
                if p.poll() is not None:
                    break
                if time.time() - start > timeout_s:
                    try: p.terminate()
                    except Exception: pass
                    try: p.kill()
                    except Exception: pass
                    return 124, True
                line = p.stdout.readline()
                if line:
                    print(line.rstrip("\r\n"))
                else:
                    time.sleep(0.05)
        except Exception:
            try: p.kill()
            except Exception: pass
            return 1, False
        try:
            rc = p.wait(timeout=1)
        except Exception:
            rc = 1
        return rc, False