"""CI-only negative controls for every captured surface; never run during R1 editing."""
import json
import unittest
from scan import MARKER, REQUIRED, scan


def record(kind, data=b"opaque", negative=False):
    return json.dumps({"event":"relay-proof", "kind":kind, "negative":negative, "bytes":data.hex()}, separators=(",", ":"))


class ScanTests(unittest.TestCase):
    def test_c12_every_surface_is_scanned(self):
        baseline = [record(kind) for kind in REQUIRED] + [record("storage", MARKER, True), record("in:pair-binary", MARKER, True)]
        self.assertEqual(scan(baseline)["negative_control_occurrences"], 2)
        for kind in REQUIRED:
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                scan(baseline + [record(kind, MARKER)])

    def test_c12_missing_negative_control_and_missing_surface_fail(self):
        with self.assertRaises(ValueError):
            scan([record(kind) for kind in REQUIRED])
        with self.assertRaises(ValueError):
            scan([record("storage", MARKER, True), record("in:pair-binary", MARKER, True)])

    def test_f8_sql_only_or_frame_only_negative_controls_are_insufficient(self):
        baseline = [record(kind) for kind in REQUIRED]
        for kind in ("storage", "in:pair-binary"):
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "SQL and frame"):
                scan(baseline + [record(kind, MARKER, True)])

    def test_c12_empty_records_cannot_satisfy_any_capture_surface(self):
        controls = [record("storage", MARKER, True), record("in:pair-binary", MARKER, True)]
        for missing in REQUIRED:
            with self.subTest(kind=missing), self.assertRaisesRegex(ValueError, "missing non-empty"):
                scan([record(kind, b"" if kind == missing else b"opaque") for kind in REQUIRED] + controls)

    def test_c12_coverage_and_frame_counts_require_nonempty_bytes(self):
        controls = [record("storage", MARKER, True), record("in:pair-binary", MARKER, True)]
        lines = controls + [record(kind, b"\x00\x01") for kind in REQUIRED]
        lines += [record(kind, b"") for kind in REQUIRED for _ in range(3)]
        proof = scan(lines)
        self.assertEqual(proof["bytes_scanned"], 2 * len(REQUIRED))
        for kind in REQUIRED:
            self.assertEqual(proof["bytes_by_kind"][kind], 2)
            self.assertEqual(proof["captures_by_kind"][kind], 1)


if __name__ == "__main__":
    unittest.main()
