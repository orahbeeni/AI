"""Asks an X window to close the way a window manager's close button does (WM_DELETE_WINDOW).

    python3 xclose.py "Window title"
"""
import ctypes, sys

x = ctypes.cdll.LoadLibrary("libX11.so.6")
Window = ctypes.c_ulong
x.XOpenDisplay.restype = ctypes.c_void_p
x.XDefaultRootWindow.restype = Window
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XInternAtom.restype = ctypes.c_ulong
x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XQueryTree.argtypes = [ctypes.c_void_p, Window, ctypes.POINTER(Window), ctypes.POINTER(Window), ctypes.POINTER(ctypes.POINTER(Window)), ctypes.POINTER(ctypes.c_uint)]
x.XFetchName.argtypes = [ctypes.c_void_p, Window, ctypes.POINTER(ctypes.c_char_p)]

class ClientMessage(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int), ("display", ctypes.c_void_p),
                ("window", Window), ("message_type", ctypes.c_ulong), ("format", ctypes.c_int), ("data", ctypes.c_long * 5)]

class XEvent(ctypes.Union):
    _fields_ = [("xclient", ClientMessage), ("pad", ctypes.c_long * 24)]

x.XSendEvent.argtypes = [ctypes.c_void_p, Window, ctypes.c_int, ctypes.c_long, ctypes.POINTER(XEvent)]

d = x.XOpenDisplay(None)
if not d:
    sys.exit("no X display")
title = sys.argv[1].encode()

def find(w):
    name = ctypes.c_char_p()
    if x.XFetchName(d, w, ctypes.byref(name)) and name.value == title:
        return w
    root, parent, kids, n = Window(), Window(), ctypes.POINTER(Window)(), ctypes.c_uint()
    if x.XQueryTree(d, w, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(kids), ctypes.byref(n)):
        for i in range(n.value):
            r = find(kids[i])
            if r:
                return r
    return None

w = find(x.XDefaultRootWindow(d))
if not w:
    sys.exit(f"no window titled {sys.argv[1]!r}")
ev = XEvent()
ev.xclient.type = 33  # ClientMessage
ev.xclient.window = w
ev.xclient.message_type = x.XInternAtom(d, b"WM_PROTOCOLS", 0)
ev.xclient.format = 32
ev.xclient.data[0] = x.XInternAtom(d, b"WM_DELETE_WINDOW", 0)
ev.xclient.data[1] = 0
x.XSendEvent(d, w, 0, 0, ctypes.byref(ev))
x.XFlush(d)
print("close requested")
