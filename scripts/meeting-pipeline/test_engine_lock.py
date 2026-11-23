"""Exercise the actual Windows shared lock, using synthetic folders only."""
from pathlib import Path
import json
import os
import subprocess
import sys
import time
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


def query_environment(root, policy="scripts"):
    """Protocol stub only; typed config parity is exercised by the native synthetic proof."""
    exe = root / 'policy.exe'
    if not exe.exists():
        setup = root / 'policy.ps1'
        setup.write_text("Add-Type -OutputType ConsoleApplication -OutputAssembly '" + str(exe) + "' -TypeDefinition @'\n" +
            'using System; class Policy { static void Main() { Console.WriteLine(Environment.GetEnvironmentVariable("SIDENOTE_SYNTHETIC_POLICY")); } }' + "\n'@")
        subprocess.run(['powershell', '-NoProfile', '-File', str(setup)], check=True,
                       capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
    return os.environ | {'SIDENOTE_DATA_DIR': str(root), 'SIDENOTE_NOTES_QUERY_EXE': str(exe),
                         'SIDENOTE_SYNTHETIC_POLICY': policy}


class EngineLockTests(unittest.TestCase):
    def test_app_off_and_unreadable_block_watcher_and_replays(self):
        with tempfile.TemporaryDirectory(prefix="notes-engine-policy-") as tmp:
            root = Path(tmp)
            (root / 'job-contract.json').write_bytes((HERE / 'job-contract.json').read_bytes())
            for name in ['hex-pipeline-cli.ps1', 'hex-meeting-watch.ps1',
                         'hex-meeting-english.ps1', 'hex-meeting-notes.ps1']:
                source = (HERE / name).read_text()
                if name == 'hex-meeting-watch.ps1':
                    source = source.replace("$root = Join-Path $env:APPDATA 'hex\\meetings'", "$root = '" + str(root / 'must-not-read') + "'")
                    source = source.replace("$log = Join-Path $env:LOCALAPPDATA 'hex-meeting-watch.log'",
                                            "$log = '" + str(root / 'watch.log') + "'")
                (root / name).write_text(source)
            absent = root / 'must-not-read'
            for engine in ['app', 'off', 'unknown', None]:
                (root / 'config.json').write_text(json.dumps(dict(notes=dict(engine=engine))) if engine else '{')
                policy = 'settings_invalid' if engine is None else ('app' if engine == 'app' else 'off')
                expected = 'notes skipped ' + ('category=' + policy if engine is None else 'engine=' + policy)
                for name, args in [('hex-meeting-watch.ps1', []),
                                   ('hex-meeting-english.ps1', ['-MeetingDir', str(absent)]),
                                   ('hex-meeting-english.ps1', ['-MeetingDir', str(absent), '-NotesOnly']),
                                   ('hex-meeting-notes.ps1', ['-InputFile', str(absent), '-OutFile', str(absent)])]:
                    result = subprocess.run(['powershell', '-NoProfile', '-File', str(root / name), *args],
                        capture_output=True, text=True, timeout=20, creationflags=subprocess.CREATE_NO_WINDOW,
                        env=query_environment(root, policy))
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stderr, "")
                    if name == 'hex-meeting-watch.ps1':
                        lines = (root / 'watch.log').read_text(encoding='utf-8-sig').splitlines()
                        self.assertEqual(len(lines), 1)
                        self.assertTrue(lines[0].endswith(' ' + expected))
                        (root / 'watch.log').unlink()
                    else:
                        self.assertEqual(result.stdout.strip(), expected)
                    self.assertFalse(absent.exists())

    def test_watcher_preserves_failure_when_deferred(self):
        with tempfile.TemporaryDirectory(prefix="notes-watcher-") as tmp:
            root = Path(tmp)
            (root / "config.json").write_text('{"notes":{"engine":"scripts"}}')
            meetings = root / "meetings"
            meeting = meetings / "1800000000000-1"
            meeting.mkdir(parents=True)
            (meeting / "manifest.json").write_text('{"status":"complete"}')
            (meeting / "transcript.ndjson").write_text('{"text":"synthetic"}')
            marker = meeting / "pipeline-failed.txt"
            marker.write_text("try synthetic failure")
            log = root / "watch.log"
            # Only locations change; the watcher control flow runs unchanged.
            source = (HERE / "hex-meeting-watch.ps1").read_text()
            source = source.replace("$root = Join-Path $env:APPDATA 'hex\\meetings'", "$root = '" + str(meetings) + "'")
            source = source.replace("$log = Join-Path $env:LOCALAPPDATA 'hex-meeting-watch.log'", "$log = '" + str(log) + "'")
            self.assertIn("$root = '" + str(meetings) + "'", source)
            source = source.replace('Local\\HexMeeting', 'Local\\Synthetic-' + root.name + '-HexMeeting')
            (root / "hex-meeting-watch.ps1").write_text(source)
            (root / "hex-pipeline-cli.ps1").write_text((HERE / "hex-pipeline-cli.ps1").read_text())
            (root / 'job-contract.json').write_bytes((HERE / 'job-contract.json').read_bytes())
            (root / "hex-meeting-english.ps1").write_text("param($MeetingDir)\nWrite-Output 'notes deferred engine busy'\nWrite-Output 'PRIVATE_SENTINEL'")
            (root / "hex-meeting-quick-title.ps1").write_text("param($MeetingDir)\nreturn")
            script = root / "run.ps1"
            script.write_text("$ErrorActionPreference='Stop'\n(Get-Item -LiteralPath '" + str(marker) +
                              "').LastWriteTimeUtc = [DateTime]::UtcNow.AddMinutes(-20)\n& '" + str(root / "hex-meeting-watch.ps1") + "'")
            result = subprocess.run(['powershell', '-NoProfile', '-File', str(script)],
                                    capture_output=True, text=True, timeout=20,
                                    creationflags=subprocess.CREATE_NO_WINDOW,
                                    env=query_environment(root))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(marker.read_text(), "try synthetic failure")
            self.assertIn("notes deferred engine busy", log.read_text(encoding="utf-8-sig"))
            self.assertNotIn("done", log.read_text(encoding="utf-8-sig"))
            self.assertNotIn("PRIVATE_SENTINEL", log.read_text(encoding="utf-8-sig"))


    def test_recycled_meeting_writers_defer_without_publication(self):
        with tempfile.TemporaryDirectory(prefix="meeting-recycle-claim-") as tmp:
            root = Path(tmp)
            meeting = root / "100-1"
            meeting.mkdir()
            (meeting / "manifest.json").write_text('{"status":"complete"}')
            original = (meeting / "manifest.json").read_bytes()
            proof = root / "proof.ps1"
            proof.write_text("""
$ErrorActionPreference = 'Stop'
. 'CLI'
$sharedA = Enter-MeetingRecycleClaim 'MEETING'
$sharedB = Enter-MeetingRecycleClaim 'MEETING/'
if ($null -eq $sharedA -or $null -eq $sharedB) { throw 'shared publication claim failed' }
$path = $sharedA.Name
if ($path -cne $sharedB.Name) { throw 'trailing slash selected a different claim' }
if ($null -ne (Enter-MeetingRecycleClaim 'MEETING/invalid')) { throw 'invalid path acquired a claim' }
$sharedA.Dispose()
$sharedB.Dispose()
$exclusive = [IO.File]::Open($path, 'Open', 'ReadWrite', 'None')
try {
    foreach ($folder in @('MEETING', 'MEETING/')) {
        if ($null -ne (Enter-MeetingRecycleClaim $folder)) { throw 'writer entered during recycle' }
    }
} finally { $exclusive.Dispose() }
$state = Join-Path $env:SIDENOTE_DATA_DIR 'local/deleted-meeting-ids.json'
[IO.File]::WriteAllText($state, '{"100-1":"synthetic"}')
foreach ($folder in @('MEETING', 'MEETING/')) {
    if ($null -ne (Enter-MeetingRecycleClaim $folder)) { throw 'deleted meeting acquired a claim' }
}
$result = & 'ENGLISH' -MeetingDir 'MEETING'
if ($result -cne 'notes deferred engine busy') { throw 'deleted meeting was admitted' }
""".replace("CLI", str(HERE / "hex-pipeline-cli.ps1")).replace("MEETING", str(meeting))
                .replace("ENGLISH", str(HERE / "hex-meeting-english.ps1")))
            result = subprocess.run(
                ['powershell', '-NoProfile', '-File', str(proof)],
                capture_output=True, text=True, timeout=20, creationflags=subprocess.CREATE_NO_WINDOW,
                env=query_environment(root))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((meeting / "manifest.json").read_bytes(), original)
            self.assertEqual(list(meeting.iterdir()), [meeting / "manifest.json"])

    def test_python_publisher_keeps_claim_after_launcher_dies(self):
        with tempfile.TemporaryDirectory(prefix="python-recycle-claim-") as tmp:
            root = Path(tmp)
            state = root / "state"
            folder = state / "local/meeting-inputs/100-1"
            folder.mkdir(parents=True)
            (folder / "english-summary.json").write_text('{"todo":[]}')
            env = os.environ | {"APPDATA": str(root), "SIDENOTE_DATA_DIR": str(state)}
            shared = state / "local/deleted-meeting-claims/100-1"
            ready, release, done = (root / name for name in ("ready", "release", "done"))
            child = root / "child.py"
            child.write_text("""
import runpy, sys, time
from pathlib import Path
helper, folder, ready, release, done = map(Path, sys.argv[1:])
guard = runpy.run_path(helper)["guarded_meeting"]
@guard
def write_claimed(folder):
    ready.touch()
    deadline = time.monotonic() + 15
    while not release.exists():
        if time.monotonic() >= deadline:
            return
        time.sleep(0.02)
    (folder / "publication.json").write_text("{}")
write_claimed(folder)
done.touch()
""")
            command = [sys.executable, '-I', str(child), str(HERE / 'hex-pipeline-run.py'),
                       str(folder), str(ready), str(release), str(done)]
            launcher = subprocess.Popen(
                [sys.executable, '-I', '-c',
                 'import subprocess,sys; subprocess.Popen(sys.argv[1:]).wait()', *command],
                env=env, creationflags=subprocess.CREATE_NO_WINDOW)

            def await_file(path):
                deadline = time.monotonic() + 10
                while not path.exists():
                    self.assertLess(time.monotonic(), deadline, f"Timed out waiting for {path.name}")
                    time.sleep(0.02)

            try:
                await_file(ready)
                launcher.kill()
                launcher.wait(timeout=10)
                probe = f"""
try {{
    $exclusive = [IO.File]::Open('{shared}', 'Open', 'ReadWrite', 'None')
}} catch {{
    if (($_.Exception.InnerException.HResult -band 0xffff) -in @(32,33)) {{ exit 0 }}
    throw
}}
$exclusive.Dispose()
exit 1
"""
                result = subprocess.run(['powershell', '-NoProfile', '-Command', probe],
                                        env=env, capture_output=True, timeout=10,
                                        creationflags=subprocess.CREATE_NO_WINDOW)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse((folder / "publication.json").exists())
            finally:
                release.touch()
                if launcher.poll() is None:
                    launcher.kill()
                    launcher.wait(timeout=10)
                await_file(done)
            self.assertEqual((folder / "publication.json").read_text(), "{}")

            tasks = [sys.executable, '-I', str(HERE / 'hex-meeting-tasks.py'), str(folder) + '/']
            subprocess.run(tasks, env=env, check=True, capture_output=True, timeout=10)
            output = folder / "tasks.json"
            self.assertEqual(json.loads(output.read_text())["tasks"], [])
            output.unlink()
            (state / "local/deleted-meeting-ids.json").write_text('{"100-1":"synthetic"}')
            subprocess.run(tasks, env=env, check=True, capture_output=True, timeout=10)
            self.assertFalse(output.exists())

    def test_contention_release_and_crash(self):
        with tempfile.TemporaryDirectory(prefix="notes-claim-") as tmp:
            root = Path(tmp)
            (root / "config.json").write_text('{"notes":{"engine":"scripts"}}')
            meeting = root / "1800000000000-1"
            meeting.mkdir()
            script = root / "proof.ps1"
            script.write_text("""
$ErrorActionPreference = 'Stop'
. 'CLI'
$claim = Enter-NotesEngine 'MEETING'
if ($null -eq $claim) { throw 'first claim failed' }
$result = & 'ENGLISH' -MeetingDir 'MEETING'
if ($result -cne 'notes deferred engine busy') { throw 'scripts did not defer' }
$claim.Dispose()
$claim = Enter-NotesEngine 'MEETING'
if ($null -eq $claim) { throw 'release failed' }
$claim.Dispose()
$info = New-Object Diagnostics.ProcessStartInfo
$info.FileName = 'powershell.exe'
$info.Arguments = '-NoProfile -File "HOLDER"'
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$child = [Diagnostics.Process]::Start($info)
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (!(Test-Path -LiteralPath 'READY')) {
        if ($child.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'holder failed' }
        Start-Sleep -Milliseconds 50
    }
    if ($null -ne (Enter-NotesEngine 'MEETING')) { throw 'cross-process claim failed' }
} finally { if (!$child.HasExited) { $child.Kill(); $child.WaitForExit() } }
$claim = Enter-NotesEngine 'MEETING'
if ($null -eq $claim) { throw 'crash left stale ownership' }
$claim.Dispose()
Write-Output 'PASS contention, release, crash expiry; notes deferred engine busy'
""".replace('CLI', str(HERE / 'hex-pipeline-cli.ps1')).replace('ENGLISH', str(HERE / 'hex-meeting-english.ps1'))
    .replace('MEETING', str(meeting)).replace('HOLDER', str(root / 'holder.ps1')).replace('READY', str(root / 'ready')))
            (root / 'holder.ps1').write_text(". '" + str(HERE / 'hex-pipeline-cli.ps1') + "'\n$claim = Enter-NotesEngine '" + str(meeting) + "'\n[IO.File]::WriteAllText('" + str(root / 'ready') + "','ready')\nStart-Sleep -Seconds 60")
            result = subprocess.run(['powershell', '-NoProfile', '-File', str(script)], capture_output=True, text=True, timeout=20, creationflags=subprocess.CREATE_NO_WINDOW,
                                    env=query_environment(root))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(list(meeting.iterdir()), [meeting / '.notes-engine.lock'])
            print(result.stdout.strip())


if __name__ == '__main__':
    unittest.main()
