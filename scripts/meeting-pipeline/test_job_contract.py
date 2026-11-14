"""Synthetic contract coverage, run by the release check's pipeline discovery."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("job_runner", Path(__file__).with_name("hex-pipeline-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class JobContractTests(unittest.TestCase):
    def test_runner_uses_the_shared_stage_and_failure_vocabulary(self):
        self.assertEqual(runner.STAGES, set(runner.CONTRACT["stages"]))
        for stage in runner.STAGES:
            runner.STAGE = stage
            for cause in runner.CATEGORIES:
                diagnostic = runner.failure(runner.PipelineFailure(cause))
                self.assertEqual(diagnostic["stage"], stage)
                self.assertEqual(diagnostic["category"], cause)

    def test_untrusted_helper_errors_never_become_failure_text(self):
        runner.STAGE = "pipeline"
        self.assertEqual(runner.category("synthetic authentication failed"), "HELPER_AUTH")
        self.assertEqual(runner.category("synthetic quota exceeded"), "HELPER_LIMIT")
        diagnostic = runner.failure(runner.PipelineFailure("synthetic private content"))
        self.assertEqual(diagnostic["category"], "other")
        self.assertNotIn("private", str(diagnostic))
