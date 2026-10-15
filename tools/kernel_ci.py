#!/usr/bin/env python3
"""Scripted QEMU session for the HolyRust kernel (SPEC Phase 6 exit criterion 3,
real-kernel track). Drives the guest over a serial socket, sends PS/2 keys via
the QEMU monitor (sendkey), takes a VGA screendump, and asserts the transcript.

Exit codes: 0 = PASS (or SKIP when qemu is absent), 1 = FAIL.
Prints 'KERNEL-CI: PASS|SKIP|FAIL …' as the last line.

D043 (Windows port): v1 used `socket.AF_UNIX` + `/tmp`, neither of which QEMU's
Windows build provides. QEMU speaks TCP on every platform, so the transport is
now TCP loopback on an ephemeral port and scratch files live in the system temp
directory. The serial chardev runs with `wait=on` so the guest does not start
writing before we are connected — over TCP the pre-connect boot banner would
otherwise be discarded outright.
"""
import os, re, shutil, socket, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TMP = tempfile.gettempdir()
PROC = None  # the running QEMU, so `fail()` can always reap it


def qemu_path():
    """qemu-system-x86_64 from PATH, else the default Windows install dir."""
    p = shutil.which("qemu-system-x86_64")
    if p:
        return p
    for cand in (
        r"C:\Program Files\qemu\qemu-system-x86_64.exe",
        r"C:\Program Files (x86)\qemu\qemu-system-x86_64.exe",
    ):
        if os.path.exists(cand):
            return cand
    return None


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def read_ppm(path):
    """Parse a binary P6 PPM (QEMU's screendump format) -> (w, h, rgb_bytes)."""
    data = open(path, "rb").read()
    if not data.startswith(b"P6"):
        raise ValueError("screendump is not a P6 PPM")
    idx = 2
    fields = []
    while len(fields) < 3:
        while data[idx:idx + 1].isspace():
            idx += 1
        if data[idx:idx + 1] == b"#":
            while data[idx:idx + 1] != b"\n":
                idx += 1
            continue
        start = idx
        while not data[idx:idx + 1].isspace():
            idx += 1
        fields.append(int(data[start:idx]))
    idx += 1  # exactly one whitespace byte after maxval
    w, h, _maxval = fields
    return w, h, data[idx:]


def pixel(body, w, x, y):
    i = (y * w + x) * 3
    return body[i], body[i + 1], body[i + 2]


def is_bright(p):
    return p[0] > 200 and p[1] > 200 and p[2] > 200


def is_red(p):
    # The EGA/VGA default palette renders colour 4 as 0xAA0000 -> (170,0,0),
    # which QEMU reports as 168. Thresholds must accept the palette we actually
    # programmed, not an idealised 255.
    return p[0] > 140 and p[1] < 80 and p[2] < 80


def is_green(p):
    return p[1] > 140 and p[0] < 80 and p[2] < 80


def is_blue(p):
    return p[2] > 140 and p[0] < 80 and p[1] < 80


def is_black(p):
    return p[0] < 50 and p[1] < 50 and p[2] < 50


def is_horizon(p):
    """Colour 9 (light blue, RGB 85/85/255) — the flight simulator's horizon.

    Unique in the 16-colour palette: colour 1 blue is (0,0,170) and never
    reaches 200 in blue, colour 11 light cyan has full green, colour 13 light
    magenta has red 170. So a match for this predicate is a horizon pixel.
    """
    return p[2] > 200 and p[0] < 130 and p[1] < 130


def horizon_y(body, w, x):
    """Mean y of the horizon pixels in column `x`, or None if absent."""
    ys = [y for y in range(480) if is_horizon(pixel(body, w, x, y))]
    if not ys:
        return None
    return sum(ys) / len(ys)


def build_kernel():
    """Native build script per host: build.sh (POSIX) / build.ps1 (Windows)."""
    kdir = os.path.join(ROOT, "kernel")
    if os.name == "nt":
        shell = shutil.which("pwsh") or shutil.which("powershell")
        if not shell:
            return 1, "", "no pwsh/powershell found to run kernel/build.ps1"
        cmd = [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "build.ps1"]
    else:
        cmd = ["./build.sh"]
    r = subprocess.run(cmd, cwd=kdir, capture_output=True, text=True)
    return r.returncode, r.stdout, r.stderr


def fail(msg):
    # Never leak a QEMU process on a failed assertion: the serial chardev's
    # logfile stays locked while it lives, which breaks the *next* run.
    global PROC
    if PROC is not None and PROC.poll() is None:
        PROC.kill()
        try:
            PROC.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
    print(f"KERNEL-CI: FAIL — {msg}")
    sys.exit(1)


def main():
    qemu = qemu_path()
    if qemu is None:
        print("KERNEL-CI: SKIP (qemu-system-x86_64 not installed; the host-side "
              "simulation in tests/kernel_sim.rs remains the portable CI deliverable)")
        sys.exit(0)

    rc, out, err = build_kernel()
    if rc != 0:
        fail("kernel build failed:\n" + out + err)

    ser_port, mon_port = free_port(), free_port()
    ppm = os.path.join(TMP, "hr-kernel-vga.ppm")
    if os.path.exists(ppm):
        os.unlink(ppm)
    # Mirror the serial traffic to a file. The guest prints "halting (reset)"
    # and immediately pulses reset, so QEMU dies with the final writes still in
    # flight; a loopback TCP teardown (RST) discards the peer's receive queue.
    # QEMU writes this log synchronously, so it survives the exit.
    serial_log = os.path.join(TMP, "hr-kernel-serial.log")
    for p in (ppm, serial_log):
        if os.path.exists(p):
            try:
                os.unlink(p)
            except OSError:
                pass  # a stale handle from a killed run must not block the new one

    qemu_stderr = open(os.path.join(TMP, "hr-kernel-qemu.err"), "w+")
    proc = subprocess.Popen([
        qemu, "-kernel", os.path.join(ROOT, "kernel", "kernel.bin"),
        "-m", "256M", "-smp", "1", "-display", "none", "-no-reboot",
        # wait=on: hold the guest until the transcript recorder is attached
        "-chardev", (f"socket,id=ser,host=127.0.0.1,port={ser_port},"
                     f"server=on,wait=on,logfile={serial_log.replace(os.sep, '/')},logappend=off"),
        "-serial", "chardev:ser",
        "-monitor", f"tcp:127.0.0.1:{mon_port},server,nowait",
    ], stdout=subprocess.DEVNULL, stderr=qemu_stderr)
    PROC = proc

    def qemu_err_tail():
        try:
            qemu_stderr.seek(0)
            return qemu_stderr.read()[-2000:]
        except OSError:
            return ""

    t0 = time.time()
    # --- connect serial ---
    ser = None
    while time.time() - t0 < 20:
        try:
            ser = socket.create_connection(("127.0.0.1", ser_port), timeout=2)
            break
        except OSError:
            ser = None
            if proc.poll() is not None:
                fail(f"qemu exited during startup (rc={proc.returncode})\n{qemu_err_tail()}")
            time.sleep(0.05)
    if ser is None:
        fail("cannot connect to serial socket")
    ser.settimeout(10)

    buf = b""

    def read_until(needle: bytes, timeout=10.0):
        nonlocal buf
        end = time.time() + timeout
        while needle not in buf:
            if time.time() > end:
                fail(f"timeout waiting for {needle!r}; buffer so far:\n"
                     f"{buf.decode(errors='replace')}\nqemu stderr:\n{qemu_err_tail()}")
            try:
                chunk = ser.recv(4096)
            except socket.timeout:
                fail(f"serial recv timeout waiting for {needle!r}")
            if not chunk:
                fail("serial closed while waiting for " + repr(needle))
            buf += chunk
        return buf


    def wait_idle(marker: bytes, timeout=30.0) -> None:
        """Wait until the guest has FINISHED, not merely been echoed.

        Looks for `marker` only after the buffer position we are at now (so the
        previous command's output cannot satisfy it), then requires the shell
        prompt after it. The prompt is printed only when the unit completes, so
        this is what actually means "the guest is done and its pixels settle".
        """
        nonlocal buf
        start = len(buf)
        end = time.time() + timeout
        while True:
            here = buf.find(marker, start)
            if here >= 0 and buf.find(b"hr> ", here + len(marker)) >= 0:
                return
            if time.time() > end:
                fail(f"guest never returned to a prompt after {marker!r}; tail:\n"
                     f"{buf[start:].decode(errors='replace')[-400:]}")
            try:
                chunk = ser.recv(4096)
            except socket.timeout:
                fail(f"serial recv timeout waiting for idle after {marker!r}")
            if not chunk:
                fail("serial closed while waiting for idle after " + repr(marker))
            buf += chunk

    def read_value(prefix: bytes, timeout=10.0) -> str:
        """Read until `prefix`, then until the rest of that line arrives.

        `read_until` returns the instant the prefix appears, so a caller that
        immediately parses the digits can race the serial stream; this waits for
        the newline that terminates the value.
        """
        nonlocal buf
        read_until(prefix, timeout)
        end = time.time() + timeout
        while True:
            start = buf.rfind(prefix)
            nl = buf.find(b"\n", start)
            if nl >= 0:
                value = buf[start + len(prefix):nl].decode(errors="replace").strip()
                # The shell echoes the command line, so the *first* thing that
                # matches `prefix` is usually the echo of the very Print that
                # produces the value. Echoes are source text and therefore carry
                # quotes, parentheses or semicolons; a value never does.
                if value and not any(c in value for c in '()";'):
                    return value
            if time.time() > end:
                fail(f"no value after {prefix!r}; tail={buf[-200:]!r}")
            chunk = ser.recv(4096)
            if not chunk:
                fail(f"serial closed while reading {prefix!r}")
            buf += chunk

    # --- boot gate: < 5 s to the HolyRust shell (SPEC Phase 6 exit 1) ---
    read_until(b"kernel: ready")
    boot_s = time.time() - t0
    if boot_s >= 5.0:
        fail(f"boot to shell took {boot_s:.2f}s (gate: <5s)")
    print(f"boot to HolyRust shell: {boot_s:.2f}s (<5s gate)")

    def mon(cmd):
        m = socket.create_connection(("127.0.0.1", mon_port), timeout=5)
        time.sleep(0.15)
        try:
            m.recv(65536)
        except OSError:
            pass
        m.sendall((cmd + "\n").encode())
        time.sleep(0.4)
        try:
            data = m.recv(1 << 20).decode(errors="replace")
        except OSError:
            data = ""
        m.close()
        return data

    def type_line(text, settle=0.8):
        ser.sendall((text + "\n").encode())
        time.sleep(settle)

    # --- in-kernel JIT of typed code ---
    type_line('Print(6*7, "\\n");')
    read_until(b"42")

    # --- live hot-patch of the running kernel's echo handler ---
    type_line('I64 KbdEcho(I64 c) { Print("[ECHO-v2!!] ", c, " (live-patched)\\n"); return 0; }')
    type_line('SimPush(33); PumpKeys();', settle=1.0)
    read_until(b"[ECHO-v2!!] 33 (live-patched)")

    # --- heap integrity after the edit (SPEC Phase 6 exit 2) ---
    type_line('Print("post-patch heap = ", HeapCheck(), "\\n");')
    read_until(b"post-patch heap = 1")

    # --- TempleOS-RS Phase 1 (SPEC_TOS T1.5/T1.6/T1.7 + exit gate G1) --------
    # The OS booted its own source out of its own filesystem; prove it lists,
    # writes, reads, measures and deletes — all through HolyRust code.
    type_line('Dir("/TOS/*.HR");')
    read_until(b"7 entries")
    for want in (b"/TOS/Files.HR", b"/TOS/Graphics.HR", b"/TOS/Sound.HR",
                 b"/TOS/Oracle.HR", b"/TOS/Editor.HR", b"/TOS/Flight.HR",
                 b"/TOS/Shell.HR"):
        if want not in buf:
            fail(f"Dir(\"/TOS/*.HR\") did not list {want.decode()}:\n"
                 + buf.decode(errors="replace"))
    print("Phase 1: Dir(\"/TOS/*.HR\") lists all seven system sources")

    # a bare command word is routed to the HolyRust shell layer (/TOS/Shell.HR)
    type_line('ls')
    read_until(b"/TOS/Doc/TempleOS-RS.txt")
    print("Phase 1: bare `ls` dispatched by /TOS/Shell.HR over every entry")

    # create -> measure -> delete, with markers unique to this session
    type_line('FWrite("/Tmp/x.HR", "Print(42);");')
    read_until(b"wrote /Tmp/x.HR (10 bytes)")
    type_line('Print("TOS-CI size=", Size("/Tmp/x.HR"), " exists=", Exists("/Tmp/x.HR"), "\\n");')
    read_until(b"TOS-CI size=10 exists=1")
    type_line('Dir("/Tmp/*");')
    read_until(b"/Tmp/x.HR")
    type_line('FDel("/Tmp/x.HR");')
    read_until(b"deleted /Tmp/x.HR")
    type_line('Print("TOS-CI gone=", Exists("/Tmp/x.HR"), " size=", Size("/Tmp/x.HR"), "\\n");')
    read_until(b"TOS-CI gone=0 size=-1")
    print("Phase 1: FWrite/Size/Exists/FDel lifecycle verified in-OS")

    # exit gate G1: hot-patch the filesystem layer, then keep using the FS.
    type_line('Print("TOS-CI patch=", HotPatch("I64 Count() { return 4242; }"), " count=", Count(), "\\n");')
    read_until(b"TOS-CI patch=0 count=4242")
    type_line('Print("TOS-CI after-patch exists=", Exists("/TOS/Shell.HR"), "\\n");')
    read_until(b"TOS-CI after-patch exists=1")
    type_line('Cat("/TOS/Doc/TempleOS-RS.txt");')
    read_until(b"the OS describing\nitself")
    type_line('Print("TOS-CI heap=", HeapCheck(), "\\n");')
    read_until(b"TOS-CI heap=1")
    print("Phase 1 (G1): /TOS/Files.HR hot-patched live; FS and heap integrity intact")

    # --- TempleOS-RS Phase 2 (SPEC_TOS T2.1/T2.2 + BC1/BC2 + exit gate G2) ---
    def screendump_pixels():
        """Capture the screen, or die honestly.

        QEMU's monitor writes to a *fixed* path, so a capture that has not landed
        yet silently reads the previous frame — which looks exactly like a frame
        where the guest drew nothing (a black 3x3 neighbourhood and all). This
        cost two separate debugging rounds: it was first mistaken for the guest
        still executing, and only reproduced under host load. Delete the old file
        first, then wait for the new one to appear and stop growing.
        """
        if os.path.exists(ppm):
            os.remove(ppm)
        mon("screendump " + ppm.replace("\\", "/"))
        deadline = time.time() + 15.0
        last = -1
        while time.time() < deadline:
            if os.path.exists(ppm):
                now = os.path.getsize(ppm)
                if now > 0 and now == last:
                    try:
                        return read_ppm(ppm)
                    except ValueError:
                        pass  # truncated mid-write; keep waiting
                last = now
            time.sleep(0.1)
        fail(f"screendump never appeared at {ppm}")

    # T2.1: enter 640x480x16 from the shell. The kernel is already in protected
    # mode, so it programs the VGA registers directly instead of calling BIOS.
    type_line('Print("TOS-CI gfx=", GfxInit(), "\\n");')
    read_until(b"TOS-CI gfx=1")
    print("Phase 2: mode 12h entered (640x480x16)")

    # BC1 + G2: drive the primitives from the shell, then assert the EXACT
    # pixels — far stronger than "the screen is not blank".
    type_line('GfxClear(0); Plot(100, 100, 15); Plot(101, 100, 4); Plot(100, 101, 2); '
              'Print("TOS-CI drawn\\n");', settle=0.5)
    wait_idle(b"TOS-CI drawn", timeout=20)
    w, h, body = screendump_pixels()
    if (w, h) != (640, 480):
        fail(f"screendump is {w}x{h}, expected 640x480 (mode 12h not active)")
    for (x, y), pred, what in (
        ((100, 100), is_bright, "white pixel at (100,100)"),
        ((101, 100), is_red, "red pixel at (101,100)"),
        ((100, 101), is_green, "green pixel at (100,101)"),
        ((99, 100), is_black, "neighbour (99,100) must stay clear"),
        ((100, 99), is_black, "neighbour (100,99) must stay clear"),
    ):
        p = pixel(body, w, x, y)
        if not pred(p):
            near = " ".join(f"({xx},{yy})={pixel(body, w, xx, yy)}"
                            for yy in range(y - 1, y + 2) for xx in range(x - 1, x + 2))
            fail(f"{what}: got {p}\n  neighbourhood: {near}")
    print("Phase 2: Plot() places exact pixels in mode 12h")

    # T2.2: the shape vocabulary lives in HolyRust (/TOS/Graphics.HR) and is
    # reached through the bare `demo` command word in /TOS/Shell.HR.
    type_line('demo', settle=1.0)
    type_line('Print("TOS-CI demo-done\\n");', settle=0.5)
    wait_idle(b"TOS-CI demo-done", timeout=30)
    w, h, body = screendump_pixels()
    if not is_blue(pixel(body, w, 300, 20)):
        fail(f"GfxDemo banner missing at (300,20): {pixel(body, w, 300, 20)}")
    if not is_red(pixel(body, w, 600, 440)):
        fail(f"GfxDemo corner marker missing at (600,440): {pixel(body, w, 600, 440)}")
    print("Phase 2: Line/Rect/FillRect/Circle/TextOut drawn from HolyRust source")

    # BC2: after the mode change the console still works — the kernel renders its
    # own output as 8x8 glyphs into the framebuffer, and guest text lands exactly
    # where asked while the rest of a cleared screen stays black.
    type_line('GfxClear(0); TextOut(200, 300, 15, "POST-GRAPHICS-CONSOLE"); '
              'Print("TOS-CI bc2\\n");', settle=0.5)
    wait_idle(b"TOS-CI bc2", timeout=20)
    w, h, body = screendump_pixels()
    console_lit = sum(1 for xx in range(0, 240) for yy in range(0, 8)
                      if is_bright(pixel(body, w, xx, yy)))
    if console_lit < 10:
        rows = []
        for y0 in range(0, 200, 8):
            n = sum(1 for yy in range(y0, y0 + 8) for xx in range(0, w)
                    if is_bright(pixel(body, w, xx, yy)))
            if n:
                rows.append(f"y={y0}:{n}")
        fail(f"console prompt not rendered after the mode change (lit={console_lit}); "
             f"bright rows: {' '.join(rows[:12])}")
    guest_lit = sum(1 for xx in range(200, 368) for yy in range(300, 308)
                    if is_bright(pixel(body, w, xx, yy)))
    if guest_lit < 30:
        fail(f"TextOut() drew nothing at (200,300): lit={guest_lit}")
    if not is_black(pixel(body, w, 620, 470)):
        fail(f"cleared screen not black at (620,470): {pixel(body, w, 620, 470)}")
    print(f"Phase 2 (BC2): console overlay + TextOut alive after mode change "
          f"(prompt {console_lit} px, text {guest_lit} px)")

    type_line('Print("TOS-CI post-gfx heap=", HeapCheck(), "\\n");')
    read_until(b"TOS-CI post-gfx heap=1")

    # --- TempleOS-RS Phase 3 (SPEC_TOS T3.x + exit gate G3) -----------------
    # G3: "Sleep(100); Time(); returns without hanging" — and the elapsed time
    # must be real, not zero: a clock that reports nothing is worse than none.
    type_line('I64 t0 = Time(); Sleep(100); I64 t1 = Time(); '
              'Print("TOS-CI sleep=", t1 - t0, "\\n");', settle=1.2)
    slept = int(read_value(b"TOS-CI sleep="))
    if slept < 100:
        fail(f"Sleep(100) returned after {slept} ms — the timer is not advancing")
    # The upper bound only catches a clock that is wildly wrong. A *polled*
    # clock expresses host scheduling as overshoot: when the emulator is
    # descheduled (this suite runs its other tests in parallel) the PIT keeps
    # counting, so one poll can absorb a multi-second stall — measured 635, 882
    # and 2069 ms across runs for the same 100 ms request. The gate is "returns
    # without hanging"; the floor above is the part that proves time advances.
    if slept > 10000:
        fail(f"Sleep(100) took {slept} ms — the clock is not millisecond-based")
    print(f"Phase 3: Sleep(100) waited {slept} ms of real time (G3)")

    type_line('Print("TOS-CI date=", Date(), "\\n");')
    date_v = int(read_value(b"TOS-CI date="))
    if not (20000101 <= date_v <= 29991231):
        fail(f"Date() returned {date_v}, which is not a YYYYMMDD calendar date")
    print(f"Phase 3: Date() = {date_v} from the CMOS clock")

    # Pointing device: move it through the QEMU monitor and watch the OS report
    # the new position — the device is polled, so this also proves polling works.
    type_line('Print("TOS-CI mouse0=", MouseX(), ",", MouseY(), "\\n");')
    _p = read_value(b"TOS-CI mouse0=").split(",")
    if len(_p) != 2:
        fail(f"could not parse the initial mouse position: {_p!r}")
    x0, y0 = int(_p[0]), int(_p[1])
    for _ in range(6):
        mon("mouse_move 20 12")
    type_line('Print("TOS-CI mouse1=", MouseX(), ",", MouseY(), "\\n");')
    _p = read_value(b"TOS-CI mouse1=").split(",")
    if len(_p) != 2:
        fail(f"could not parse the moved mouse position: {_p!r}")
    x1, y1 = int(_p[0]), int(_p[1])
    if x1 <= x0 or y1 <= y0:
        fail(f"mouse_move did not move the pointer: ({x0},{y0}) -> ({x1},{y1})")
    print(f"Phase 3: PS/2 mouse moved ({x0},{y0}) -> ({x1},{y1})")

    # The cursor must be visible on screen: fill black, nudge the mouse so the
    # next poll redraws it, then look for the crosshair at the position the OS
    # reports. Bounded retry: the guest only repaints the cursor while it is idle
    # in its input loop, so a loaded machine can take a moment.
    type_line('GfxClear(0);', settle=1.0)
    mon("mouse_move 3 3")
    type_line('Print("TOS-CI mouse2=", MouseX(), ",", MouseY(), "\\n");', settle=0.5)
    _p = read_value(b"TOS-CI mouse2=").split(",")
    if len(_p) != 2:
        fail(f"could not parse the post-clear mouse position: {_p!r}")
    x2, y2 = int(_p[0]), int(_p[1])
    seen = False
    for _ in range(6):
        w, h, body = screendump_pixels()
        if is_bright(pixel(body, w, x2, y2)):
            seen = True
            break
        time.sleep(0.6)
    if not seen:
        fail(f"no cursor drawn at the reported position ({x2},{y2}): "
             f"got {pixel(body, w, x2, y2)}")
    print(f"Phase 3: cursor visible at ({x2},{y2}) on a cleared screen")

    # --- TempleOS-RS Phase 4 (SPEC_TOS T4.x + exit gate G4) -----------------
    # G4 accepts "at least a serial log of frequency changes": the kernel writes
    # every tone through hr_out *before* it waits, so the ordered transcript is
    # the evidence that a frequency was really programmed.
    type_line('Print("TOS-CI beep=", Beep(440, 120), "\\n");', settle=1.5)
    read_until(b"TOS-CI beep=1")
    read_until(b"[tone] 440 Hz for 120 ms", timeout=6)
    print("Phase 4: Beep(440,120) programmed the speaker and logged the tone")

    # A tune written in HolyRust drives consecutive *different* frequencies.
    type_line('Music(262, 330, 392, 523);', settle=2.5)
    for needle in (b"[tone] 262 Hz", b"[tone] 330 Hz", b"[tone] 392 Hz", b"[tone] 523 Hz"):
        read_until(needle, timeout=6)
    print("Phase 4: Music(4 notes) from /TOS/Sound.HR logged 4 distinct tones")

    # The bare `beep` command word reaches Scale() in /TOS/Sound.HR.
    type_line('beep', settle=3.5)
    read_until(b"[tone] 262 Hz for 150 ms", timeout=10)
    print("Phase 4: bare `beep` command plays Scale()")

    # The God-speaker: the entropy source must be real and in range...
    type_line('Print("TOS-CI rand=", Random(10), "\\n");')
    rv = int(read_value(b"TOS-CI rand="))
    if not 0 <= rv < 10:
        fail(f"Random(10) returned {rv}, outside 0..9")
    print(f"Phase 4: Random(10) = {rv} (in range)")

    # ...and the voice must actually vary. Eight draws over eight verses: the
    # chance of a single distinct line is 8^-7, so two is a safe floor.
    before = len(buf)
    # type_line() only writes; buf is filled by read_until(), so the block ends
    # with a marker to read to — otherwise the verses would never reach `buf`.
    type_line('OracleN(8); Print("TOS-CI oracle-done\\n");', settle=2.5)
    read_until(b"TOS-CI oracle-done", timeout=10)
    tail = buf[before:].decode(errors="replace")
    verses = ["shepherd", "meek", "darkness", "Ask, and", "merry heart",
              "To every thing", "Be still", "rooftops"]
    hits = sum(1 for v in verses if v in tail)
    if hits < 2:
        fail(f"the oracle did not vary: {hits} distinct verses in 8 draws:\n{tail}")
    print(f"Phase 4: oracle spoke {hits} distinct verses in 8 draws (G4)")

    # --- TempleOS-RS Phase 5 (SPEC_TOS T5.x + exit gate G5) -----------------
    # THE END-TO-END TEST: boot -> Ed() the OS's own source -> change a function
    # -> #include it back -> the OS's behaviour changes, the filesystem still
    # works, and memory integrity holds. This is the thesis, executed.
    type_line('Print("TOS-CI ed-open=", EdOpen("/TOS/Files.HR"), "\\n");', settle=0.8)
    ed_lines = int(read_value(b"TOS-CI ed-open="))
    if ed_lines < 50:
        fail(f"the OS's own source opened as {ed_lines} lines — expected a real file")
    print(f"Phase 5: editor opened /TOS/Files.HR ({ed_lines} lines)")

    type_line('Print("TOS-CI ed-before=", OsName(), "\\n");', settle=0.8)
    before = read_value(b"TOS-CI ed-before=")
    if before != "TempleOS-RS":
        fail(f"unexpected OsName() before the edit: {before!r}")

    # Find the one-line function, rewrite exactly that line, and save — all in
    # one unit, reporting the index and both results.
    type_line('I64 ln = EdFind("Str OsName"); '
              'Print("TOS-CI ed-edit=", ln, ":", EdSet(ln, "Str OsName() { return \\"self-edited\\"; }"), "\\n");',
              settle=1.0)
    edit = read_value(b"TOS-CI ed-edit=")
    try:
        idx_s, ok_s = edit.split(":")
        idx, ok = int(idx_s), int(ok_s)
    except ValueError:
        fail(f"could not parse the edit result: {edit!r}")
    if idx < 0 or ok != 1:
        fail(f"editing the OS's own source failed (line={idx}, set={ok})")

    type_line('Print("TOS-CI ed-save=", EdSave(), "\\n");', settle=0.8)
    if read_value(b"TOS-CI ed-save=") != "1":
        fail("EdSave() did not write the edited source back")
    print(f"Phase 5: rewrote line {idx} of the OS's own source and saved it")

    # Re-include the file: the new definition becomes the running one.
    type_line('#include "/TOS/Files.HR"\nPrint("TOS-CI reinstated\\n");', settle=0.5)
    wait_idle(b"TOS-CI reinstated", timeout=30)

    type_line('Print("TOS-CI ed-after=", OsName(), "\\n");', settle=0.8)
    after = read_value(b"TOS-CI ed-after=")
    if after != "self-edited":
        fail(f"the re-included source did not take effect: {after!r}")

    # ...and the system is still whole.
    type_line('Print("TOS-CI ed-fs=", Exists("/TOS/Shell.HR"), ":", Count(), ":", HeapCheck(), "\\n");',
              settle=0.8)
    state = read_value(b"TOS-CI ed-fs=")
    if state != "1:8:1":
        fail(f"after the self-edit: exists:count:heap = {state} (expected 1:8:1)")
    print("Phase 5 (G5): OS edited its own source, re-included it, and kept running")

    # --- font orientation (found via the Phase 7 HUD, D060) ------------------
    # Every earlier graphics assertion counted lit pixels, and a mirrored glyph
    # has exactly the same count — so five phases of screenshots showed mirrored
    # text and the suite never noticed. Read the shape instead: '/' is a forward
    # slash, so its top row lights the right half and its bottom row the left.
    type_line('GfxClear(0); GfxChar(100, 100, 15, 47); Print("TOS-CI glyph\\n");', settle=0.5)
    wait_idle(b"TOS-CI glyph", timeout=20)
    w, h, body = screendump_pixels()
    top = [x for x in range(100, 108) if is_bright(pixel(body, w, x, 100))]
    bottom = [x for x in range(100, 108) if is_bright(pixel(body, w, x, 106))]
    if not top or not bottom:
        fail(f"the '/' glyph did not render (top={top} bottom={bottom})")
    if min(top) < 104 or max(bottom) > 103:
        fail(f"glyphs are mirrored: '/' lights top x={top}, bottom x={bottom} "
             f"(a forward slash needs the top on the right and the bottom on the left)")
    print("Phase 7 (font): '/' leans forward, so glyphs are not mirrored")

    # --- TempleOS-RS Phase 7 (AfterEgypt-style flight, F20) ------------------
    # Enter flight mode, then prove the *pixels* respond to simulated mouse
    # deltas: the horizon must be level in level flight, tilt when the mouse
    # banks the aircraft, and move when the mouse pitches it.
    type_line('FlightInit();', settle=0.6)
    type_line('FlightStep(); Print("TOS-CI fly1\\n");', settle=0.6)
    wait_idle(b"TOS-CI fly1", timeout=40)
    w, h, body = screendump_pixels()
    l0 = horizon_y(body, w, 250)
    r0 = horizon_y(body, w, 390)
    c0 = horizon_y(body, w, 320)
    if l0 is None or r0 is None or c0 is None:
        fail(f"the horizon was not drawn (left={l0} right={r0} centre={c0})")
    if abs(r0 - l0) > 4:
        fail(f"a level aircraft drew a banked horizon (left={l0} right={r0})")
    print(f"Phase 7: flight mode entered; horizon level at y={c0:.0f}")

    # Mouse X banks the aircraft. Two monitor deltas of +60 px.
    mon("mouse_move 60 0")
    mon("mouse_move 60 0")
    type_line('FlightStep(); FlightStep(); Print("TOS-CI fly2\\n");', settle=0.6)
    wait_idle(b"TOS-CI fly2", timeout=40)
    w, h, body = screendump_pixels()
    l1 = horizon_y(body, w, 250)
    r1 = horizon_y(body, w, 390)
    if l1 is None or r1 is None:
        fail(f"the horizon vanished after banking (left={l1} right={r1})")
    tilt = r1 - l1
    if abs(tilt) < 5:
        fail(f"banking did not tilt the horizon (left={l1:.0f} right={r1:.0f}, tilt={tilt:.0f})")
    print(f"Phase 7: mouse X banked the aircraft; horizon tilt {tilt:+.0f} px")

    # Mouse Y pitches it: the horizon must move vertically.
    mon("mouse_move 0 60")
    type_line('FlightStep(); Print("TOS-CI fly3\\n");', settle=0.6)
    wait_idle(b"TOS-CI fly3", timeout=40)
    w, h, body = screendump_pixels()
    c1 = horizon_y(body, w, 320)
    if c1 is None:
        fail("the horizon vanished after pitching")
    if abs(c1 - c0) < 5:
        fail(f"pitching did not move the horizon (before={c0:.0f} after={c1:.0f})")
    print(f"Phase 7: mouse Y pitched the aircraft; horizon moved {c1 - c0:+.0f} px")

    # Exit cleanly, and the OS must still be whole afterwards.
    type_line('FlightExit(); Print("TOS-CI landed=", HeapCheck(), ":", Count(), "\\n");', settle=0.6)
    landed = read_value(b"TOS-CI landed=")
    if landed != "1:8":
        fail(f"after landing: heap:count = {landed} (expected 1:8)")
    print("Phase 7 (G7): flight entered, horizon tracked the mouse, exit clean")




    # --- PS/2 keyboard path via monitor sendkey: type `42` + Enter ---
    mon("sendkey 4"); mon("sendkey 2"); mon("sendkey ret")
    # shell echoes typed chars after the prompt, then the expression value
    read_until(b"hr> 42")
    if buf.count(b"42") < 2:
        fail(f"PS/2 sendkey did not produce the `42` line; transcript:\n{buf.decode(errors='replace')}")

    # --- VGA screendump must show the (now graphical) console ----------------
    w, h, body = screendump_pixels()
    nonblank = sum(1 for i in range(0, len(body) - 2, 3)
                   if body[i] > 40 or body[i + 1] > 40 or body[i + 2] > 40)
    if nonblank < 100:
        fail(f"VGA screen blank ({nonblank} lit px)")
    print(f"VGA screendump: {nonblank} lit pixels ({w}x{h})")

    # --- clean shutdown via #quit (reset pulse + -no-reboot) ---
    type_line("#quit")
    # The guest prints "kernel: halting (reset)" and *then* pulses reset, so
    # QEMU exits microseconds later. Read continuously rather than waiting for
    # the process first: a TCP chardev's final writes are otherwise lost with
    # the teardown (this was D043 failure mode #2).
    ser.settimeout(0.3)
    deadline = time.time() + 15
    while time.time() < deadline:
        try:
            chunk = ser.recv(65536)
        except socket.timeout:
            if proc.poll() is not None:
                break
            continue
        except OSError:
            break
        if not chunk:
            break
        buf += chunk
    try:
        rc = proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        fail("qemu did not exit after #quit")

    # --- transcript ordering ---
    t = buf.replace(b"\r\n", b"\n").decode(errors="replace")
    # Prefer QEMU's chardev log when it captured the reset tail the socket lost.
    if os.path.exists(serial_log):
        try:
            logged = open(serial_log, "rb").read().decode(errors="replace")
            if "halting (reset)" in logged:
                t = logged.replace("\r\n", "\n")
        except OSError:
            pass
    marks = ["S2:64bit-longmode", "compiler runs IN the kernel", "heap integrity: 1",
             "[echo-v1] 104", "[echo-v1] 105", "kernel: ready", "42",
             "[ECHO-v2!!] 33 (live-patched)", "post-patch heap = 1", "halting (reset)"]
    idx = -1
    for m in marks:
        j = t.find(m)
        if j < 0:
            fail(f"transcript missing {m!r}\n---\n{t}")
        if j < idx:
            fail(f"transcript out of order at {m!r}\n---\n{t}")
        idx = j
    print("transcript: all markers present, in order")
    print(f"KERNEL-CI: PASS (boot {boot_s:.2f}s, qemu rc={rc})")


if __name__ == "__main__":
    main()
