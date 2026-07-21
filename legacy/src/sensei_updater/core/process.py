import subprocess, sys, time
from .colors import MAGENTA, DIM, RESET, GRAY

class Process:
    def __init__(self, debug: bool=False, dry_run: bool=False):
        self.debug = debug
        self.dry_run = dry_run

    def run_stream(self, cmd: list[int|str]) -> int:
        if self.debug: print(f"{MAGENTA}{DIM}>>> {' '.join(map(str,cmd))}{RESET}")
        if self.dry_run:
            print(f"{GRAY}[dry-run]{RESET} {' '.join(map(str,cmd))}")
            return 0
        try:
            p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace", shell=False)
            assert p.stdout is not None
            for line in p.stdout:
                if line.strip().startswith("VERBOSE:"):
                    print(f"{GRAY}{line.rstrip()}{RESET}")
                else:
                    print(line, end="")
            return p.wait()
        except KeyboardInterrupt:
            try: p.terminate()
            except Exception: pass
            return 1

    def run_capture(self, cmd: list[int|str]) -> tuple[int,str]:
        if self.debug: print(f"{MAGENTA}{DIM}>>> {' '.join(map(str,cmd))}{RESET}")
        if self.dry_run: return 0, ""
        r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", shell=False)
        return r.returncode, r.stdout

    def run_capture_timeout(self, cmd: list[int|str], timeout_s: float) -> tuple[int,str,bool]:
        if self.debug: print(f"{MAGENTA}{DIM}>>> {' '.join(map(str,cmd))}{RESET}")
        if self.dry_run: return 0, "", False
        start = time.time()
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace", shell=False)
        out = []
        try:
            while True:
                if p.poll() is not None:
                    break
                if time.time() - start > timeout_s:
                    try: p.terminate()
                    except Exception: pass
                    return 124, "".join(out), True
                chunk = p.stdout.readline() if p.stdout else ""
                if chunk:
                    out.append(chunk)
                else:
                    time.sleep(0.05)
        finally:
            try:
                rc = p.wait(timeout=1)
            except Exception:
                rc = 124
        return rc, "".join(out), False

    def run_stream_progress(self, cmd: list[int|str], label: str, idle_anim: str="|/-\\", idle_tick: float=0.1, idle_after: float=1.0) -> int:
        if self.debug: print(f"{MAGENTA}{DIM}>>> {' '.join(map(str,cmd))}{RESET}")
        if self.dry_run:
            print(f"{GRAY}[dry-run]{RESET} {label}")
            return 0
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace", shell=False)
        assert p.stdout is not None
        last = time.time()
        frame = 0
        try:
            while True:
                line = p.stdout.readline()
                if line:
                    print(line, end="")
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
        finally:
            try:
                rc = p.wait(timeout=1)
            except Exception:
                rc = 1
            sys.stdout.write("\r" + " " * (len(label) + 2) + "\r")
            sys.stdout.flush()
        return rc