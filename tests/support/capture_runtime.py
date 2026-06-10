#!/usr/bin/env python3

import os
import shlex
import subprocess
import sys
import tempfile
import time


def tmux(socket: str, *arguments: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["tmux", "-L", socket, *arguments],
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: capture_runtime.py <yater> <epub>", file=sys.stderr)
        return 2

    socket = f"yater-runtime-{os.getpid()}"
    session = "reader"
    server_started = False
    with tempfile.TemporaryDirectory() as runtime_home, tempfile.NamedTemporaryFile() as capture:
        environment = (
            "TERM=xterm-256color "
            f"XDG_DATA_HOME={shlex.quote(runtime_home + '/data')} "
            f"XDG_STATE_HOME={shlex.quote(runtime_home + '/state')} "
        )
        command = (
            f"{environment}exec {shlex.quote(sys.argv[1])} "
            f"{shlex.quote(sys.argv[2])} --image-mode=off"
        )

        try:
            try:
                tmux(
                    socket,
                    "new-session",
                    "-d",
                    "-x",
                    "40",
                    "-y",
                    "8",
                    "-s",
                    session,
                    "/bin/sh",
                )
                server_started = True
            except (FileNotFoundError, subprocess.CalledProcessError) as error:
                print(f"tmux unavailable: {error}", file=sys.stderr)
                return 77
            tmux(
                socket,
                "pipe-pane",
                "-t",
                session,
                "-o",
                f"cat > {shlex.quote(capture.name)}",
            )
            tmux(socket, "send-keys", "-t", session, command, "Enter")

            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                pane = tmux(socket, "capture-pane", "-p", "-t", session).stdout
                if b"Opening heading." in pane:
                    break
                time.sleep(0.05)
            else:
                print(
                    f"reader did not render its first frame: {pane!r}",
                    file=sys.stderr,
                )
                return 124

            time.sleep(1)
            tmux(socket, "send-keys", "-t", session, "-l", "q")
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if tmux(socket, "has-session", "-t", session, check=False).returncode != 0:
                    break
                time.sleep(0.05)
            else:
                pane = tmux(socket, "capture-pane", "-p", "-t", session).stdout
                print(f"reader did not exit after q: {pane!r}", file=sys.stderr)
                return 124

            capture.seek(0)
            sys.stdout.buffer.write(capture.read())
            return 0
        finally:
            if server_started:
                tmux(socket, "kill-server", check=False)


if __name__ == "__main__":
    raise SystemExit(main())
