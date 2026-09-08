import os
import subprocess
import sys
import unittest
from pathlib import Path

from boot import MARKERS, run


class BootTests(unittest.TestCase):
    def simulate(self, case, timeout=5):
        return run(
            [sys.executable, str(Path(__file__).with_name("fake_qemu.py")), case],
            timeout,
        )

    def test_success_requires_acknowledged_shutdown(self):
        ok, serial, reason = self.simulate("pass")
        self.assertTrue(ok, reason)
        self.assertIn(b"TEST_RESULT: PASS\r\n", serial)

    def test_fragmented_serial(self):
        ok, _, reason = self.simulate("fragmented")
        self.assertTrue(ok, reason)

    def test_every_stage_is_required_exactly_once(self):
        for case in (*[f"omit-{index}" for index in range(len(MARKERS))], "duplicate"):
            with self.subTest(case=case):
                ok, _, reason = self.simulate(case)
                self.assertFalse(ok)
                self.assertTrue(reason)

    def test_rejects_incomplete_boot_and_failed_shutdown(self):
        for case in (
            "exit-zero",
            "exit-error",
            "marker-then-exit",
            "missing",
            "out-of-order",
            "substring",
            "unterminated",
            "failed",
            "bad-greeting",
            "quit-error",
            "quit-no-ack",
            "quit-nonzero",
            "overflow",
            "late-fail",
        ):
            with self.subTest(case=case):
                ok, _, reason = self.simulate(case)
                self.assertFalse(ok)
                self.assertTrue(reason)

    def test_boot_timeout(self):
        ok, _, reason = self.simulate("hang", timeout=0.25)
        self.assertFalse(ok)
        self.assertIn("timeout", reason)

    def test_shutdown_timeout(self):
        for case in ("quit-hang", "quit-ack-hang"):
            with self.subTest(case=case):
                ok, serial, reason = self.simulate(case, timeout=1)
                self.assertFalse(ok)
                self.assertIn("timeout", reason)
                pid = int(serial.splitlines()[0].removeprefix(b"PID: "))
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)

    def test_launch_failure(self):
        ok, serial, reason = run(["/nonexistent/core-qemu"], 1)
        self.assertFalse(ok)
        self.assertEqual(serial, b"")
        self.assertTrue(reason)

    def test_cli_exit_status_and_serial_capture(self):
        directory = Path(__file__).parent
        for case, expected in (("pass", 0), ("quit-nonzero", 1)):
            with self.subTest(case=case):
                result = subprocess.run(
                    [
                        sys.executable,
                        str(directory / "boot.py"),
                        "--timeout",
                        "5",
                        "--",
                        sys.executable,
                        str(directory / "fake_qemu.py"),
                        case,
                    ],
                    capture_output=True,
                    timeout=10,
                    check=False,
                )
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertIn(b"TEST_RESULT: PASS", result.stdout)
                self.assertNotIn(b"\r", result.stdout)


if __name__ == "__main__":
    unittest.main()
