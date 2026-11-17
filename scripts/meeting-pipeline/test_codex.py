"""Production entry points with synthetic native CLIs, never account access."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CodexTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='hex-fake-cli-')
        cls.root = Path(cls.temp.name)
        source = cls.root / 'fake.cs'
        source.write_text(r'''
using System;
using System.IO;
class Fake {
 static void Main(string[] args) {
  string name = Path.GetFileNameWithoutExtension(Environment.GetCommandLineArgs()[0]);
  string log = Environment.GetEnvironmentVariable("FAKE_LOG");
  File.AppendAllText(log, name + "\n" + String.Join("\n", args) + "\nEND\n");
  Console.In.ReadToEnd();
  if (name == "claude") { Console.Write("interactive answer"); return; }
  if (Environment.GetEnvironmentVariable("FAKE_ERROR") != null) {
   Console.Error.Write(Environment.GetEnvironmentVariable("FAKE_ERROR"));
   Environment.Exit(37);
  }
  string answer = Environment.GetEnvironmentVariable("FAKE_ANSWER");
  int i = Array.IndexOf(args, "--output-last-message");
  if (i < 0) Environment.Exit(38);
  File.WriteAllText(args[i+1], answer);
  Console.WriteLine("{\"type\":\"thread.started\"}");
 }
}''')
        setup = cls.root / 'compile.ps1'
        setup.write_text("Add-Type -OutputType ConsoleApplication -OutputAssembly '" +
                         str(cls.root / 'codex.exe') + "' -Path '" + str(source) + "'\n")
        subprocess.run(['powershell', '-NoProfile', '-File', str(setup)],
                       check=True, capture_output=True)
        (cls.root / 'claude.exe').write_bytes((cls.root / 'codex.exe').read_bytes())

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix='hex-codex-test-')
        self.addCleanup(self.work.cleanup)
        self.folder = Path(self.work.name)
        self.log = self.folder / 'calls.txt'
        self.env = os.environ | dict(PATH=str(self.root) + os.pathsep + os.environ['PATH'],
            APPDATA=str(self.folder), SIDENOTE_DATA_DIR=str(self.folder),
            FAKE_LOG=str(self.log), FAKE_ANSWER='Synthetic planning meeting')

    def assert_isolated(self):
        calls = self.log.read_text()
        self.assertNotIn('claude\n', calls)
        for flag in ['exec', '--ignore-user-config', '--ephemeral', '--skip-git-repo-check',
                     '--sandbox\nread-only', 'features.hooks=false',
                     'features.shell_tool=false', 'mcp_servers={}', '--output-last-message']:
            self.assertIn(flag, calls)

    def test_codex_request_and_interactive_claude(self):
        runner = load('hex-pipeline-run')
        request = self.folder / 'request.json'
        prompt, output = self.folder / 'input.txt', self.folder / 'output.txt'
        prompt.write_text('Synthetic input')
        request.write_text(json.dumps(dict(kind='codex', stage='notes', model='gpt-6.1-sol',
            timeout=5, input_file=str(prompt), output_file=str(output))))
        with patch.dict(os.environ, self.env):
            runner.run_request(request)
            self.assertEqual(output.read_text(), self.env['FAKE_ANSWER'])
            self.assert_isolated()
            self.assertEqual(runner.claude('Synthetic input', 'synthetic-model', 5), 'interactive answer')

    def test_codex_failure_contract(self):
        runner = load('hex-pipeline-run')
        for message, expected in [('Your refresh token has expired', 'HELPER_AUTH'),
                                  ('Please run codex login', 'HELPER_AUTH'),
                                  ("You've hit your usage limit", 'HELPER_LIMIT')]:
            with self.subTest(expected=expected), patch.dict(os.environ, self.env | {'FAKE_ERROR': message}):
                with self.assertRaises(runner.PipelineFailure) as caught:
                    runner.codex('Synthetic', 'gpt-6.1-sol', 5)
                self.assertEqual(caught.exception.category, expected)
                self.assertEqual(caught.exception.kind, 'codex')

    def test_automatic_tasks_and_compare(self):
        with patch.dict(os.environ, self.env):
            tasks = load('hex-meeting-tasks')
            summary = dict(title='Synthetic', todo=[dict(task='Synthetic task')])
            (self.folder / 'english-summary.json').write_text(json.dumps(summary))
            os.environ['FAKE_ANSWER'] = json.dumps(dict(meeting_project='Synthetic', tasks=[]))
            tasks.classify(self.folder)
            compare = load('hex-meeting-compare-granola')
            self.assertEqual(compare.codex('Synthetic')[0]['meeting_project'], 'Synthetic')
        self.assert_isolated()

    def test_automatic_notes_and_quick_title(self):
        from test_engine_lock import query_environment
        env = self.env | query_environment(self.folder)
        env.update(self.env)
        meeting = self.folder / '1800000000000-1'
        meeting.mkdir()
        (meeting / 'transcript.ndjson').write_text('{"text":"Synthetic discussion","start_ms":0}')
        prompt = self.folder / 'prompt.txt'
        prompt.write_text('# Instructions\nSynthetic\n# Input\n## Transcript\nSynthetic')
        env['FAKE_ANSWER'] = json.dumps(dict(title='Synthetic', context='', topics=[], todo=[], to_settle=[]))
        for name, args in [('hex-meeting-notes.ps1', ['-InputFile', str(prompt), '-OutFile',
                str(meeting / 'english-summary.json'), '-MeetingDir', str(meeting), '-Project', 'other']),
                ('hex-meeting-quick-title.ps1', ['-MeetingDir', str(meeting)])]:
            result = subprocess.run(['powershell', '-NoProfile', '-File', str(HERE / name), *args],
                env=env, capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((meeting / 'quick-title.json').is_file())
        self.assert_isolated()


if __name__ == '__main__':
    unittest.main()
