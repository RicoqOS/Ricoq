"""The QEMU contract requires allocation, mapping, verification, then unmapping."""

import sys
import unittest
from pathlib import Path

from boot import MARKERS, run


class VSpaceTests(unittest.TestCase):
    def test_vspace_contract(self):
        self.assertEqual(
            MARKERS[4:],
            (
                b"vspace: frame allocated",
                b"vspace: frame mapped",
                b"vspace: memory verified",
                b"vspace: frame unmapped",
                b"vspace: image restored",
                b"TEST_RESULT: PASS",
            ),
        )

    def test_rejects_missing_or_reordered_vspace_steps(self):
        for case in (
            "omit-4", "omit-5", "omit-6", "omit-7", "omit-8",
            "vspace-out-of-order", "vspace-legacy",
        ):
            with self.subTest(case=case):
                ok, _, reason = run(
                    [sys.executable, str(Path(__file__).with_name("fake_qemu.py")), case],
                    5,
                )
                self.assertFalse(ok)
                self.assertEqual(reason, "unexpected boot marker order")
