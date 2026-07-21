RESET = "\x1b[0m"
BOLD = "\x1b[1m"
DIM = "\x1b[2m"
RED = "\x1b[31m"
GREEN = "\x1b[32m"
YELLOW = "\x1b[33m"
MAGENTA = "\x1b[35m"
CYAN = "\x1b[36m"

def C256(n: int) -> str:
    return f"\x1b[38;5;{n}m"

ORANGE2 = C256(208)
ORANGE1 = C256(214)
SUN     = C256(226)
BROWN   = C256(94)
AMBER   = C256(178)
GRAY    = C256(245)
WHITE   = C256(255)