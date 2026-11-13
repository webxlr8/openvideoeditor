"""Asks the app's top-level X11 windows to close, as the window manager's close button does (WM_DELETE_WINDOW).

Usage: python3 close_window.py <window title>
Exits 0 when at least one window was asked, 1 when none was found. Needs python3-xlib and $DISPLAY.
WebDriver's own Close Window ends the session by killing the app, which skips its normal shutdown; this does not.
"""

import sys

from Xlib import X, display, protocol


def windows(d, root):
    stack = [root]
    while stack:
        w = stack.pop()
        yield w
        try:
            stack.extend(w.query_tree().children)
        except Exception:  # a window can vanish while the tree is read
            pass


def main():
    title = sys.argv[1]
    d = display.Display()
    protocols = d.intern_atom("WM_PROTOCOLS")
    delete = d.intern_atom("WM_DELETE_WINDOW")
    asked = 0
    for w in windows(d, d.screen().root):
        try:
            if w.get_wm_name() != title or delete not in (w.get_wm_protocols() or []):
                continue
        except Exception:
            continue
        ev = protocol.event.ClientMessage(window=w, client_type=protocols, data=(32, [delete, X.CurrentTime, 0, 0, 0]))
        w.send_event(ev, event_mask=X.NoEventMask)
        asked += 1
    d.flush()
    d.close()
    sys.exit(0 if asked else 1)


if __name__ == "__main__":
    main()
