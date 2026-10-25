#!/usr/bin/env python3
"""Source-only candidate: phase-A private-X11 rendered handoff observer.

Must run only under the separately admitted display/resource guardian. This
does not start a display, install anything, claim focus, or relax that guardian.
"""
import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import stat
import struct
import subprocess
import time
import zlib


APP_ID = b"io.github.zzwong.Diffz"
GEOMETRY = (20, 20, 1100, 700)
# Interior reader pixels, excluding title/toolbar/file tree/footer/scrollbar.
ROI = (330, 140, 700, 300)
DEADLINE = 0.0


class Attributes(C.Structure):
    _fields_ = [(n, t) for n, t in [
        ("x", C.c_int), ("y", C.c_int), ("width", C.c_int),
        ("height", C.c_int), ("border_width", C.c_int), ("depth", C.c_int),
        ("visual", C.c_void_p), ("root", C.c_ulong), ("class_", C.c_int),
        ("bit_gravity", C.c_int), ("win_gravity", C.c_int),
        ("backing_store", C.c_int), ("backing_planes", C.c_ulong),
        ("backing_pixel", C.c_ulong), ("save_under", C.c_int),
        ("colormap", C.c_ulong), ("map_installed", C.c_int),
        ("map_state", C.c_int), ("all_event_masks", C.c_long),
        ("your_event_mask", C.c_long), ("do_not_propagate_mask", C.c_long),
        ("override_redirect", C.c_int), ("screen", C.c_void_p),
    ]]


class XImage(C.Structure):
    _fields_ = [("width", C.c_int), ("height", C.c_int),
                ("xoffset", C.c_int), ("format", C.c_int),
                ("data", C.c_void_p), ("byte_order", C.c_int),
                ("bitmap_unit", C.c_int), ("bitmap_bit_order", C.c_int),
                ("bitmap_pad", C.c_int), ("depth", C.c_int),
                ("bytes_per_line", C.c_int), ("bits_per_pixel", C.c_int),
                ("red_mask", C.c_ulong), ("green_mask", C.c_ulong),
                ("blue_mask", C.c_ulong), ("obdata", C.c_void_p),
                ("functions", C.c_void_p * 6)]


def need(condition, message):
    if not condition:
        raise RuntimeError(message)


def remaining():
    value = DEADLINE - time.monotonic()
    need(value > 0, "observer deadline reached")
    return value


def wait_for(fn, description, seconds=30):
    until = min(DEADLINE, time.monotonic() + seconds)
    while time.monotonic() < until:
        value = fn()
        if value:
            return value
        time.sleep(0.25)
    raise RuntimeError("timed out: " + description)


class Display:
    def __init__(self, library):
        self.lib = C.CDLL(str(library))
        declarations = {
            "XOpenDisplay": ([C.c_char_p], C.c_void_p),
            "XCloseDisplay": ([C.c_void_p], C.c_int),
            "XDefaultRootWindow": ([C.c_void_p], C.c_ulong),
            "XInternAtom": ([C.c_void_p, C.c_char_p, C.c_int], C.c_ulong),
            "XFree": ([C.c_void_p], C.c_int),
            "XSync": ([C.c_void_p, C.c_int], C.c_int),
            "XQueryTree": ([C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong),
                            C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)),
                            C.POINTER(C.c_uint)], C.c_int),
            "XGetWindowProperty": ([C.c_void_p, C.c_ulong, C.c_ulong,
                C.c_long, C.c_long, C.c_int, C.c_ulong, C.POINTER(C.c_ulong),
                C.POINTER(C.c_int), C.POINTER(C.c_ulong), C.POINTER(C.c_ulong),
                C.POINTER(C.POINTER(C.c_ubyte))], C.c_int),
            "XGetWindowAttributes": ([C.c_void_p, C.c_ulong, C.POINTER(Attributes)], C.c_int),
            "XMoveResizeWindow": ([C.c_void_p, C.c_ulong, C.c_int, C.c_int,
                                    C.c_uint, C.c_uint], C.c_int),
            "XGetImage": ([C.c_void_p, C.c_ulong, C.c_int, C.c_int,
                           C.c_uint, C.c_uint, C.c_ulong, C.c_int], C.POINTER(XImage)),
            "XDestroyImage": ([C.POINTER(XImage)], C.c_int),
        }
        for name, (args, result) in declarations.items():
            fn = getattr(self.lib, name)
            fn.argtypes, fn.restype = args, result
        self.errors = 0
        callback = C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)
        def on_error(_display, _event):
            self.errors += 1
            return 0
        self.error_handler = callback(on_error)
        self.lib.XSetErrorHandler.argtypes = [callback]
        self.lib.XSetErrorHandler.restype = C.c_void_p
        self.lib.XSetErrorHandler(self.error_handler)
        self.handle = self.lib.XOpenDisplay(None)
        need(self.handle, "could not connect to admitted private display")
        self.root = self.lib.XDefaultRootWindow(self.handle)

    def sync(self):
        self.lib.XSync(self.handle, 0)
        need(self.errors == 0, "X11 protocol error; observation invalid")

    def property(self, window, name):
        atom = self.lib.XInternAtom(self.handle, name.encode(), 1)
        if not atom:
            return None
        actual, fmt, count, after = C.c_ulong(), C.c_int(), C.c_ulong(), C.c_ulong()
        data = C.POINTER(C.c_ubyte)()
        status = self.lib.XGetWindowProperty(self.handle, window, atom, 0, 4096,
            0, 0, C.byref(actual), C.byref(fmt), C.byref(count), C.byref(after), C.byref(data))
        try:
            need(status == 0 and after.value == 0, "invalid or oversized X property")
            if not actual.value:
                return None
            if fmt.value == 8:
                return C.string_at(data, count.value)
            if fmt.value == 32:
                return list(C.cast(data, C.POINTER(C.c_ulong))[:count.value])
            raise RuntimeError("unsupported X property encoding")
        finally:
            if data:
                self.lib.XFree(data)

    def children(self):
        root, parent, count = C.c_ulong(), C.c_ulong(), C.c_uint()
        children = C.POINTER(C.c_ulong)()
        need(self.lib.XQueryTree(self.handle, self.root, C.byref(root),
            C.byref(parent), C.byref(children), C.byref(count)), "XQueryTree failed")
        try:
            need(count.value <= 128, "unexpected private-display window count")
            return list(children[:count.value])
        finally:
            if children:
                self.lib.XFree(children)

    def attributes(self, window):
        attr = Attributes()
        need(self.lib.XGetWindowAttributes(self.handle, window, C.byref(attr)),
             "owned window no longer exists")
        return attr

    def find(self, state, binary):
        matches = []
        for window in self.children():
            cls = self.property(window, "WM_CLASS")
            if not isinstance(cls, bytes) or APP_ID not in cls.split(b"\0"):
                continue
            values = self.property(window, "_NET_WM_PID")
            need(isinstance(values, list) and len(values) == 1, "app window lacks PID")
            try:
                ident = identity(values[0], state, binary)
            except (FileNotFoundError, ProcessLookupError):
                continue
            if ident and self.attributes(window).map_state == 2:
                matches.append((window, ident))
        self.sync()
        need(len(matches) <= 1, "multiple app windows for one private state")
        return matches[0] if matches else None

    def capture(self, window, rectangle):
        x, y, width, height = rectangle
        attr = self.attributes(window)
        need(attr.map_state == 2 and x >= 0 and y >= 0 and
             x + width <= attr.width and y + height <= attr.height,
             "capture is not inside mapped client window")
        image = self.lib.XGetImage(self.handle, window, x, y, width, height,
                                   C.c_ulong(-1).value, 2)
        need(image, "XGetImage failed")
        try:
            im = image.contents
            need(im.width == width and im.height == height and im.xoffset == 0 and
                 im.format == 2 and im.bits_per_pixel in (16, 24, 32) and
                 im.byte_order in (0, 1), "unsupported X image format")
            stride, step = im.bytes_per_line, im.bits_per_pixel // 8
            need(width * step <= stride <= 8192 and height <= 900, "invalid image stride")
            channels = []
            for mask in (im.red_mask, im.green_mask, im.blue_mask):
                need(mask > 0, "missing TrueColor channel")
                shift = (mask & -mask).bit_length() - 1
                maximum = mask >> shift
                need(maximum & (maximum + 1) == 0, "noncontiguous color mask")
                channels.append((mask, shift, maximum))
            raw = C.string_at(im.data, stride * height)
            rgb = bytearray(width * height * 3)
            order = "little" if im.byte_order == 0 else "big"
            out = 0
            for row in range(height):
                for col in range(width):
                    offset = row * stride + col * step
                    pixel = int.from_bytes(raw[offset:offset + step], order)
                    for mask, shift, maximum in channels:
                        rgb[out] = (((pixel & mask) >> shift) * 255 + maximum // 2) // maximum
                        out += 1
            self.sync()
            return bytes(rgb)
        finally:
            self.lib.XDestroyImage(image)


def identity(pid, state, binary):
    proc = Path("/proc") / str(pid)
    fields = (proc / "stat").read_text().rsplit(")", 1)[1].split()
    need(fields[0] != "Z", "app process became a zombie")
    status = (proc / "status").read_text()
    uid = int(re.search(r"^Uid:\s+(\d+)", status, re.M).group(1))
    need(uid == os.getuid(), "app process has a different UID")
    args = (proc / "cmdline").read_bytes().split(b"\0")
    pairs = list(zip(args, args[1:]))
    if (b"--state-dir", os.fsencode(state)) not in pairs:
        return None
    need((proc / "exe").resolve() == binary, "app process executable mismatch")
    return {"pid": pid, "start_ticks": int(fields[19]), "uid": uid}


def png(path, width, height, rgb):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + rgb[y * width * 3:(y + 1) * width * 3] for y in range(height))
    payload = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    payload += chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
    path.write_bytes(payload)
    return hashlib.sha256(payload).hexdigest()


def invoke(binary, state, fixture, expected, root, label):
    need("XDG_ACTIVATION_TOKEN" not in os.environ, "phase A requires absent activation metadata")
    env = dict(os.environ, DIFFZ_TIMING="1")
    result = subprocess.run([str(binary), "--state-dir", str(state), "--fixture", fixture, "--json"],
        env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=min(20, remaining()), check=False)
    (root / (label + ".stdout")).write_bytes(result.stdout)
    (root / (label + ".stderr")).write_bytes(result.stderr)
    need(result.returncode == 0, label + " launcher failed")
    answer = json.loads(result.stdout.decode().strip().splitlines()[-1])
    need(answer.get("status") == expected, label + " unexpected transport status")
    return answer


def ready(display, binary, state):
    window, ident = wait_for(lambda: display.find(state, binary), "app mapped window")
    display.lib.XMoveResizeWindow(display.handle, window, *GEOMETRY)
    display.sync()
    def painted():
        log = state / "diffz.log"
        return log.is_file() and b"diffz-timing first content paint " in log.read_bytes()
    wait_for(painted, "first active-snapshot content paint")
    wait_for(lambda: (display.attributes(window).width, display.attributes(window).height) ==
             GEOMETRY[2:], "fixed client geometry")
    return window, ident


def stable_crop(display, window, expected=None):
    previous, since = None, 0.0
    def sample():
        nonlocal previous, since
        pixels = display.capture(window, ROI)
        if pixels != previous or (expected is not None and pixels != expected):
            previous, since = pixels, time.monotonic()
            return None
        if time.monotonic() - since < 1.0:
            return None
        # Reject uniform/blank surfaces, but never call this text recognition.
        colors = set(zip(pixels[0::3], pixels[1::3], pixels[2::3]))
        need(len(colors) >= 16, "reader crop is blank or insufficiently rendered")
        return pixels
    return wait_for(sample, "stable rendered reader reference", seconds=40)


def snapshot(display, window, root, label):
    width, height = GEOMETRY[2:]
    pixels = display.capture(window, (0, 0, width, height))
    path = root / (label + ".png")
    return {"path": str(path), "sha256": png(path, width, height, pixels)}


def run(args, result):
    root, binary, library = args.run_root.resolve(), args.binary.resolve(), args.x11_library.resolve()
    need(root.is_dir() and root.stat().st_uid == os.getuid() and
         stat.S_IMODE(root.stat().st_mode) == 0o700, "run root is not private and owned")
    need(binary.parent == root and binary.is_file() and os.access(binary, os.X_OK),
         "binary is not guardian's executable copy")
    need(library == args.x11_library and library.is_file(), "X11 library must be canonical")
    need(os.environ.get("XAUTHORITY") == str(root / "client.auth"), "wrong private Xauthority")
    display_record = json.loads((root / "display-ready.json").read_text())
    need(re.fullmatch(r":\d+", os.environ.get("DISPLAY", "")) and
         display_record.get("display") == os.environ["DISPLAY"], "private display receipt mismatch")
    reference_state, target_state = root / "reference-state", root / "target-state"
    for state_dir in (reference_state, target_state):
        need(len(os.fsencode(state_dir / "handoff.sock")) < 104, "state socket path too long")
        state_dir.mkdir(mode=0o700)
    display = Display(library)
    try:
        need(display.property(display.root, "_NET_SUPPORTING_WM_CHECK") is None,
             "phase A requires the admitted no-WM display")
        result["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        result["geometry"] = list(GEOMETRY)
        result["reader_roi"] = list(ROI)
        result["reference_transport"] = invoke(binary, reference_state, "F02", "launched", root, "reference-F02")
        reference_window, reference_identity = ready(display, binary, reference_state)
        reference = stable_crop(display, reference_window)
        result["reference"] = {"xid": reference_window, **reference_identity,
            "crop_sha256": hashlib.sha256(reference).hexdigest(),
            "image": snapshot(display, reference_window, root, "reference-F02")}
        need(hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"),
             "pidfd ownership-safe reference termination is unavailable")
        # Pin the process first, then verify its recorded PID/starttime/UID/exe/state.
        # No numeric-PID signaling fallback if this capability is unavailable.
        reference_fd = os.pidfd_open(reference_identity["pid"], 0)
        try:
            need(identity(reference_identity["pid"], reference_state, binary) == reference_identity,
                 "reference process ownership changed before stop")
            signal.pidfd_send_signal(reference_fd, signal.SIGTERM, None, 0)
        finally:
            os.close(reference_fd)
        wait_for(lambda: reference_window not in display.children(), "reference window termination", seconds=5)
        wait_for(lambda: not (Path("/proc") / str(reference_identity["pid"])).exists(),
                 "guardian reaped reference process", seconds=5)
        result["initial_transport"] = invoke(binary, target_state, "F01", "launched", root, "target-F01")
        window, initial_identity = ready(display, binary, target_state)
        initial = stable_crop(display, window)
        changed_pixels = sum(a != b for a, b in zip(
            zip(initial[0::3], initial[1::3], initial[2::3]),
            zip(reference[0::3], reference[1::3], reference[2::3])))
        need(changed_pixels >= ROI[2] * ROI[3] // 100, "F01/F02 reader references are not distinguishable")
        result["before"] = {"xid": window, **initial_identity,
            "crop_sha256": hashlib.sha256(initial).hexdigest(),
            "image": snapshot(display, window, root, "before-F01")}
        result["handoff_transport"] = invoke(binary, target_state, "F02", "handed_off", root, "handoff-F02")
        final = stable_crop(display, window, expected=reference)
        final_match = display.find(target_state, binary)
        need(final_match == (window, initial_identity), "handoff replaced process or window")
        attr = display.attributes(window)
        need((attr.width, attr.height) == GEOMETRY[2:], "handoff changed client geometry")
        result["after"] = {"xid": window, **initial_identity,
            "crop_sha256": hashlib.sha256(final).hexdigest(),
            "image": snapshot(display, window, root, "after-F02")}
        result.update({"phase_a_passed": True, "same_pid_starttime_xid": True,
            "reader_changed_pixels": changed_pixels,
            "matches_independent_f02_render": True,
            "focus_verified": False, "wayland_token_verified": False,
            "content_text_independently_read": False,
            "scope": "Real X11 tokenless handoff, with exact rendered reader comparison to independent cold F02. PNG inspection remains required; no focus/compositor-policy or Wayland-token qualification."})
    finally:
        display.lib.XCloseDisplay(display.handle)


def main():
    global DEADLINE
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-root", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--x11-library", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    DEADLINE = time.monotonic() + 210
    result = {"schema_version": 1, "phase_a_passed": False, "focus_verified": False,
              "wayland_token_verified": False, "content_text_independently_read": False}
    code = 1
    try:
        run(args, result)
        code = 0
    except Exception as error:
        result["error"] = str(error)
    # The guardian owns termination/reaping of every remaining descendant.
    (args.run_root / "observer-result.json").write_text(json.dumps(result, indent=2) + "\n")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
