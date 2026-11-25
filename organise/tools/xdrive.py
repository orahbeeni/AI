"""Sends real mouse input to an X display (for UI tests under Xvfb).

    python3 xdrive.py "move 100 200" "down" "glide 100 120 8" "up" "click 50 60" "sleep 0.5" "rclick 10 10"
    python3 xdrive.py "key Home" "key BackSpace" "key ctrl+z"     # X keysym names; + for modifiers
"""
import ctypes, sys, time

x11 = ctypes.cdll.LoadLibrary("libX11.so.6")
xt = ctypes.cdll.LoadLibrary("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
d = ctypes.c_void_p(x11.XOpenDisplay(None))
if not d.value:
    sys.exit("no X display")
pos = [0, 0]

def move(x, y):
    xt.XTestFakeMotionEvent(d, -1, int(x), int(y), 0)
    x11.XFlush(d)
    pos[:] = [x, y]

x11.XStringToKeysym.restype = ctypes.c_ulong
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XKeysymToKeycode.restype = ctypes.c_ubyte
MODS = {"ctrl": "Control_L", "shift": "Shift_L", "alt": "Alt_L"}

def key(combo):
    codes = []
    for name in combo.split("+"):
        ks = x11.XStringToKeysym(MODS.get(name.lower(), name).encode())
        kc = x11.XKeysymToKeycode(d, ks)
        if not kc:
            sys.exit(f"unknown key {name}")
        codes.append(kc)
    for kc in codes:
        xt.XTestFakeKeyEvent(d, kc, 1, 0)
    for kc in reversed(codes):
        xt.XTestFakeKeyEvent(d, kc, 0, 0)
    x11.XFlush(d)

def button(n, down):
    xt.XTestFakeButtonEvent(d, n, 1 if down else 0, 0)
    x11.XFlush(d)

for cmd in sys.argv[1:]:
    a = cmd.split()
    if a[0] == "move":
        move(float(a[1]), float(a[2]))
    elif a[0] == "down":
        button(1, True)
    elif a[0] == "up":
        button(1, False)
    elif a[0] == "glide":  # glide x y steps: move there in small steps, like a hand
        x0, y0 = pos
        n = int(a[3]) if len(a) > 3 else 10
        for i in range(1, n + 1):
            move(x0 + (float(a[1]) - x0) * i / n, y0 + (float(a[2]) - y0) * i / n)
            time.sleep(0.03)
    elif a[0] in ("click", "rclick"):
        move(float(a[1]), float(a[2]))
        time.sleep(0.1)
        b = 1 if a[0] == "click" else 3
        button(b, True)
        time.sleep(0.05)
        button(b, False)
    elif a[0] == "key":
        key(a[1])
    elif a[0] == "sleep":
        time.sleep(float(a[1]))
    time.sleep(0.05)
