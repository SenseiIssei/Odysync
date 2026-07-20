import sys, threading, time, itertools

class Spinner:
    def __init__(self, prefix=""):
        self.prefix = prefix
        self._stop = threading.Event()
        self._t = None

    def start(self, text=""):
        msg = f"{self.prefix}{text}" if text else self.prefix
        self._stop.clear()
        self._t = threading.Thread(target=self._run, args=(msg,), daemon=True)
        self._t.start()

    def _run(self, msg):
        frames = itertools.cycle(["|", "/", "-", "\\"])
        while not self._stop.is_set():
            sys.stdout.write(f"\r{msg} {next(frames)}")
            sys.stdout.flush()
            time.sleep(0.1)
        sys.stdout.write("\r" + " " * (len(msg) + 2) + "\r")
        sys.stdout.flush()

    def stop(self):
        if self._t:
            self._stop.set()
            self._t.join(timeout=1)