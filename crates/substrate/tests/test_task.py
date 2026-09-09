"""The QEMU contract requires generic construction and isolation checks."""

import sys
import unittest
from pathlib import Path

from boot import MARKERS, run


class TaskTests(unittest.TestCase):
    def test_task_contract(self):
        self.assertEqual(
            MARKERS[1:],
            (
                b"task: resources constructed",
                b"task: cspaces and vspaces isolated",
                b"task: capability isolation verified",
                b"task: independent execution verified",
                b"task: private memory isolation verified",
                b"task: per-task IPC buffers verified",
                b"TEST_RESULT: PASS",
            ),
        )

    def test_rejects_missing_or_reordered_task_steps(self):
        for case in (
            "omit-1",
            "omit-2",
            "omit-3",
            "omit-4",
            "omit-5",
            "omit-6",
            "task-out-of-order",
            "task-legacy",
        ):
            with self.subTest(case=case):
                ok, _, reason = run(
                    [
                        sys.executable,
                        str(Path(__file__).with_name("fake_qemu.py")),
                        case,
                    ],
                    5,
                )
                self.assertFalse(ok)
                self.assertEqual(reason, "unexpected boot marker order")
