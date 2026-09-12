"""Controlled child process for testing the host protocol and failure cleanup."""

import json
import os
import signal
import socket
import sys


def emit(data):
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()


def main():
    case = sys.argv[1]
    address = sys.argv[sys.argv.index("-qmp") + 1].split(",")[0][5:]
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
        server.bind(address)
        server.listen(1)
        emit(f"PID: {os.getpid()}\n".encode())
        if case == "hang":
            signal.pause()
        if case.startswith("exit-"):
            return 0 if case == "exit-zero" else 7
        markers = [
            b"substrate: booting",
            b"task: resources constructed",
            b"task: cspaces and vspaces isolated",
            b"task: capability isolation verified",
            b"task: independent execution verified",
            b"task: private memory isolation verified",
            b"task: per-task IPC buffers verified",
            b"ipc: basic request reply verified",
            b"ipc: deferred badged replies verified",
            b"ipc: capability transfer used and tracked",
            b"fault: routes distinguished",
            b"fault: VM context decoded",
            b"fault: register control verified",
            b"fault: VM recovery continued",
            b"fault: unknown syscall decoded",
            b"TEST_RESULT: PASS",
        ]
        if case.startswith("omit-"):
            del markers[int(case.removeprefix("omit-"))]
        elif case == "duplicate":
            markers.insert(1, markers[0])
        elif case == "task-out-of-order":
            markers[5], markers[6] = markers[6], markers[5]
        elif case == "task-legacy":
            del markers[1:-1]
        serial = b"\r\n".join(markers) + b"\r\n"
        prefix = b"\n".join(markers[:-1]) + b"\n"
        if case == "missing":
            serial = prefix
        elif case == "out-of-order":
            serial = b"TEST_RESULT: PASS\n" + prefix
        elif case == "substring":
            serial = prefix + b"not TEST_RESULT: PASS\n"
        elif case == "unterminated":
            serial = prefix + b"TEST_RESULT: PASS"
        elif case == "failed":
            serial = prefix + b"TEST_RESULT: FAIL\n"
        elif case == "overflow":
            serial = b"x" * (1024 * 1024 + 1)
        if case == "fragmented":
            for byte in serial:
                emit(bytes([byte]))
        else:
            emit(serial)
        if case.startswith("omit-") or case == "duplicate" or case in (
            "missing",
            "out-of-order",
            "substring",
            "unterminated",
            "failed",
            "overflow",
            "marker-then-exit",
            "task-out-of-order",
            "task-legacy",
        ):
            return 0
        with server.accept()[0] as connection:
            with connection.makefile("rwb", buffering=0) as stream:
                greeting = {"wrong": {}} if case == "bad-greeting" else {"QMP": {}}
                stream.write(json.dumps(greeting).encode() + b"\n")
                for line in stream:
                    request = json.loads(line)
                    quitting = request["execute"] == "quit"
                    if quitting and case == "quit-hang":
                        signal.pause()
                    if quitting and case == "quit-no-ack":
                        return 0
                    reply = {"return": {}, "id": request["id"]}
                    if quitting and case == "quit-error":
                        reply = {"error": {}, "id": request["id"]}
                    stream.write(json.dumps(reply).encode() + b"\n")
                    if quitting and case == "quit-ack-hang":
                        signal.pause()
                    if quitting and case == "late-fail":
                        emit(b"TEST_RESULT: FAIL\n")
                    if quitting:
                        return 7 if case == "quit-nonzero" else 0
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (BrokenPipeError, ConnectionResetError):
        os._exit(8)
