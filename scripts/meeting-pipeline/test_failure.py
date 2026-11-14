"""Synthetic children only: exercise runner, wrapper and diagnostic allowlists."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("runner", HERE / "hex-pipeline-run.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class FailureTests(unittest.TestCase):
    def test_fixed_patterns(self):
        for text, expected in [("usage limit", "HELPER_LIMIT"),
                               ("You have hit your limit", "HELPER_LIMIT"),
                               ("usage_limit_reached", "HELPER_LIMIT"),
                               ("UNAUTHORIZED", "HELPER_AUTH"), ("private payload", "other")]:
            self.assertEqual(runner.category(text), expected)
        for name in ("git", "claude", "codex", "python"):
            self.assertEqual(runner.child_kind([name + ".exe"]), name)

    def test_real_runner_and_wrapper(self):
        with tempfile.TemporaryDirectory(prefix="notes-failure-") as tmp:
            root = Path(tmp)
            child = root / "child.py"
            child.write_text("import sys; sys.stderr.write('usage limit PRIVATE_SENTINEL'); sys.exit(37)")
            request = root / "request.json"
            diagnostic = root / "failure.json"
            request.write_text(json.dumps(dict(kind="native", stage="notes", timeout=5,
                args=[sys.executable, str(child)], cwd=tmp)))
            result = subprocess.run([sys.executable, "-I", str(HERE / "hex-pipeline-run.py"),
                "--request", str(request), "--diagnostic", str(diagnostic)], capture_output=True)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout + result.stderr, b"")
            data = json.loads(diagnostic.read_text())
            self.assertEqual((data["kind"], data["exit_code"], data["category"]),
                             ("python", 37, "HELPER_LIMIT"))
            self.assertNotIn("PRIVATE_SENTINEL", diagnostic.read_text())
            script = root / "wrapper.ps1"
            script.write_text(". '" + str(HERE / "hex-pipeline-cli.ps1") + "'\n" +
                "try { Invoke-HexCli -Stage notes -CliArgs @('" + sys.executable + "','" +
                str(child) + "') -WorkingDirectory '" + tmp +
                "' -TimeoutSeconds 5; exit 9 } catch { Write-Output $_.Exception.Message; exit 37 }")
            wrapped = subprocess.run(["powershell", "-NoProfile", "-File", str(script)],
                                     capture_output=True, text=True)
            self.assertEqual(wrapped.returncode, 37, wrapped.stderr)
            self.assertRegex(wrapped.stdout, r"pipeline-failure stage=notes category=HELPER_LIMIT kind=python exit_code=37 deadline_seconds=5 elapsed_seconds=[0-9.]+")
            self.assertNotIn("PRIVATE_SENTINEL", wrapped.stdout + wrapped.stderr)
            print(wrapped.stdout.strip() + " runner_exit=1 wrapper_exit=37")

    def test_timeout_and_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            for args, timeout, expected in [([sys.executable, "-c", "import time; time.sleep(3)"], .05, "timeout"),
                                            ([str(Path(tmp) / "git.exe")], 1, "other")]:
                with self.assertRaises(runner.PipelineFailure) as caught:
                    runner.execute(args, tmp, timeout)
                self.assertEqual(caught.exception.category, expected)
                self.assertIn(caught.exception.kind, runner.KINDS)


if __name__ == "__main__":
    unittest.main()
