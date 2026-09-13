#!/usr/bin/env python3
"""Exercise the real TUI input path in an isolated PTY (macOS/Linux).

Builds the test binary, sends SGR mouse events, and records calls to the system
opener instead of launching a browser. No running Workbench session is touched.
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[1]
HAND = b"\x1b]22;pointer\x1b\\"
DEFAULT = b"\x1b]22;default\x1b\\"


def main():
    build = subprocess.run(
        ["cargo", "test", "--offline", "--bin", "workbench", "--no-run", "--message-format=json-render-diagnostics"],
        cwd=ROOT, text=True, stdout=subprocess.PIPE, check=True,
    )
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith("{")]
    binary = next(a["executable"] for a in artifacts
                  if a.get("executable") and a.get("profile", {}).get("test"))
    with tempfile.TemporaryDirectory(prefix="workbench-links-test-") as temp:
        folder = Path(temp)
        opened = folder / "opened"
        for name in ("open", "xdg-open"):
            launcher = folder / name
            launcher.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "$WORKBENCH_TEST_OPEN_LOG"\n')
            launcher.chmod(0o700)
        env = dict(os.environ, PATH=f"{folder}{os.pathsep}{os.environ['PATH']}",
                   TERM="xterm-ghostty", TERM_PROGRAM="ghostty",
                   WORKBENCH_LINK_FIXTURE=str(folder), WORKBENCH_TEST_OPEN_LOG=str(opened))
        env.pop("TMUX", None)
        child, master = pty.fork()
        if child == 0:
            os.chdir(ROOT)
            os.execve(binary, [binary, "links::fixture::terminal", "--exact", "--ignored", "--nocapture"], env)
        output = bytearray()
        reaped = False

        def pump():
            if select.select([master], [], [], 0.02)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    pass

        def wait_for(check, description):
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                pump()
                result = check()
                if result:
                    return result
            raise AssertionError(f"Timed out: {description}; terminal tail: {bytes(output[-1500:])!r}")

        def frame():
            try:
                return json.loads((folder / "frame.json").read_text())
            except FileNotFoundError:
                return {}

        def calls():
            return opened.read_text().splitlines() if opened.exists() else []

        def mouse(button, hit, release=False, dx=0):
            os.write(master, f"\x1b[<{button};{hit['x'] + 1 + dx};{hit['y'] + 1}{'m' if release else 'M'}".encode())

        def pointer_after(action, expected):
            start = len(output)
            action()
            wait_for(lambda: expected in output[start:], "pointer update")

        try:
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
            hits = wait_for(lambda: frame().get("hits"), "first rendered links")
            hidden, plain = hits[:2]
            pointer_after(lambda: mouse(35, hidden), HAND)
            mouse(0, hidden)
            wait_for(lambda: frame().get("event") == f"MouseClick({hidden['x']}, {hidden['y']})", "mouse press")
            assert not frame()["highlighted"], "a click must not flash a selected character"
            mouse(0, hidden, release=True)
            wait_for(lambda: calls() == [hidden["url"]], "embedded link opener invocation")

            mouse(0, hidden)
            mouse(32, hidden, dx=4)
            mouse(0, hidden, release=True, dx=4)
            wait_for(lambda: frame().get("selection"), "drag selection")
            assert calls() == [hidden["url"]], "drag must not open a URL"

            mouse(0, plain)
            mouse(0, plain, release=True)
            wait_for(lambda: calls() == [hidden["url"], plain["url"]], "plain URL opener invocation")
            pointer_after(lambda: mouse(35, {"x": 0, "y": 0}), DEFAULT)
            pointer_after(lambda: mouse(35, hidden), HAND)
            pointer_after(lambda: os.write(master, b"\x10"), DEFAULT)
            wait_for(lambda: frame().get("mode") == "CommandPalette", "palette opens")
            mouse(0, hidden)
            mouse(0, hidden, release=True)
            # A subsequent move is processed after the click/release pair.
            mouse(35, {"x": 0, "y": 0})
            wait_for(lambda: frame().get("event") == "MouseMove(0, 0)", "modal click ignored")
            assert len(calls()) == 2, "links behind a modal must not open"
            os.write(master, b"\x1b")
            wait_for(lambda: frame().get("mode") == "Normal", "palette closes")
            pointer_after(lambda: mouse(35, hidden), HAND)

            def resize():
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 90, 0, 0))
                os.kill(child, signal.SIGWINCH)

            pointer_after(resize, DEFAULT)
            new_hits = wait_for(lambda: frame().get("hits") if frame().get("hits") and frame()["hits"][0]["x"] != hidden["x"] else None, "resized link positions")
            pointer_after(lambda: mouse(35, new_hits[0]), HAND)
            mouse(0, new_hits[0])
            mouse(0, new_hits[0], release=True)
            wait_for(lambda: len(calls()) == 3, "link click after resize")
            pointer_after(lambda: os.write(master, b"\x11"), DEFAULT)
            def exited():
                status = os.waitpid(child, os.WNOHANG)
                return status if status[0] else None

            status = wait_for(exited, "fixture exits")
            reaped = True
            assert os.waitstatus_to_exitcode(status[1]) == 0, bytes(output[-1500:])
            print("Passed: real PTY hover, embedded/plain URL clicks, drag selection, modal blocking, resize, and pointer cleanup")
        finally:
            if not reaped:
                (folder / "stop").touch()
                try:
                    os.kill(child, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                os.waitpid(child, 0)
            os.close(master)


if __name__ == "__main__":
    main()
