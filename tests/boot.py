"""Bounded serial boot test with an acknowledged, host-requested QEMU shutdown."""

import argparse
import json
import selectors
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


OUTPUT_LIMIT = 1024 * 1024
MARKERS = (
    b"substrate: booting",
    b"substrate: cspace ready",
    b"substrate: untyped ready",
    b"substrate: notification allocated",
    b"TEST_RESULT: PASS",
)


class BootError(Exception):
    pass


def remaining(deadline):
    seconds = deadline - time.monotonic()
    if seconds <= 0:
        raise BootError("timeout waiting for boot or shutdown")
    return seconds


def quit_qemu(address, process, deadline):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(remaining(deadline))
        connection.connect(address)
        pending = bytearray()

        def receive():
            while b"\n" not in pending:
                connection.settimeout(remaining(deadline))
                chunk = connection.recv(4096)
                if not chunk:
                    raise BootError("missing QMP response")
                pending.extend(chunk)
                if len(pending) > 65536:
                    raise BootError("missing or oversized QMP response")
            line, _, rest = pending.partition(b"\n")
            pending[:] = rest
            response = json.loads(line)
            if not isinstance(response, dict):
                raise BootError("invalid QMP response")
            return response

        if "QMP" not in receive():
            raise BootError("missing QMP greeting")
        for command in ("qmp_capabilities", "quit"):
            if process.poll() is not None:
                raise BootError("QEMU exited before the shutdown request")
            connection.settimeout(remaining(deadline))
            connection.sendall(
                json.dumps(
                    {
                        "execute": command,
                        "id": command,
                    }
                ).encode()
                + b"\n"
            )
            # QMP can interleave asynchronous events with command replies.
            for _ in range(32):
                response = receive()
                if "event" in response:
                    continue
                if response.get("id") != command or "return" not in response:
                    raise BootError(f"QMP {command} was not acknowledged")
                break
            else:
                raise BootError("too many QMP events")


def run(command, timeout):
    serial = bytearray()
    process = None
    deadline = time.monotonic() + timeout
    try:
        # A short path also fits Darwin's smaller Unix-domain socket path limit.
        with tempfile.TemporaryDirectory(prefix="boot-", dir="/tmp") as directory:
            address = str(Path(directory) / "qmp")
            process = subprocess.Popen(
                command + ["-qmp", f"unix:{address},server=on,wait=off"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                bufsize=0,
            )
            with process.stdout as output, selectors.DefaultSelector() as selector:
                selector.register(output, selectors.EVENT_READ)
                pending = bytearray()
                marker_index = 0
                quitting = False
                while True:
                    if not selector.select(remaining(deadline)):
                        raise BootError("timeout waiting for serial output")
                    chunk = output.read(4096)
                    if not chunk:
                        if not quitting:
                            raise BootError("QEMU serial EOF before expected shutdown")
                        break
                    available = OUTPUT_LIMIT - len(serial)
                    serial.extend(chunk[:available])
                    if len(chunk) > available:
                        raise BootError("serial output exceeded 1 MiB")
                    pending.extend(chunk)
                    while b"\n" in pending:
                        line, _, rest = pending.partition(b"\n")
                        pending = bytearray(rest)
                        line = line.removesuffix(b"\r")
                        if line in MARKERS:
                            if (
                                marker_index >= len(MARKERS)
                                or line != MARKERS[marker_index]
                            ):
                                raise BootError("unexpected boot marker order")
                            marker_index += 1
                        elif line.startswith(b"TEST_RESULT:"):
                            raise BootError("guest reported an unexpected test result")
                    if marker_index == len(MARKERS) and not quitting:
                        quit_qemu(address, process, deadline)
                        quitting = True
                status = process.wait(timeout=remaining(deadline))
                if status != 0:
                    raise BootError(f"QEMU exited with status {status}")
                return True, bytes(serial), "boot sequence and QEMU shutdown passed"
    except (BootError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        reason = (
            "timeout waiting for boot or shutdown"
            if isinstance(error, (TimeoutError, subprocess.TimeoutExpired))
            else str(error)
        )
        return False, bytes(serial), reason
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command or not 0 < args.timeout <= 3600:
        parser.error("a command and timeout in (0, 3600] are required")
    ok, serial, reason = run(command, args.timeout)
    # Nix's build-log renderer treats carriage returns as line replacement.
    sys.stdout.buffer.write(serial.replace(b"\r", b""))
    sys.stdout.buffer.flush()
    print(f"BOOT_HARNESS: {'PASS' if ok else 'FAIL'}: {reason}", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
