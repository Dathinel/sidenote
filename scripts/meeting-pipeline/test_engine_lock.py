"""Exercise the actual Windows shared lock, using synthetic folders only."""
from pathlib import Path
import json
import os
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


class EngineLockTests(unittest.TestCase):
    def test_app_off_and_unreadable_block_watcher_and_replays(self):
        with tempfile.TemporaryDirectory(prefix="notes-engine-policy-") as tmp:
            root = Path(tmp)
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
                expected = 'notes skipped engine=' + ('app' if engine == 'app' else 'off')
                for name, args in [('hex-meeting-watch.ps1', []),
                                   ('hex-meeting-english.ps1', ['-MeetingDir', str(absent)]),
                                   ('hex-meeting-english.ps1', ['-MeetingDir', str(absent), '-NotesOnly']),
                                   ('hex-meeting-notes.ps1', ['-InputFile', str(absent), '-OutFile', str(absent)])]:
                    result = subprocess.run(['powershell', '-NoProfile', '-File', str(root / name), *args],
                        capture_output=True, text=True, timeout=20, creationflags=subprocess.CREATE_NO_WINDOW,
                        env=os.environ | {'SIDENOTE_DATA_DIR': str(root)})
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
            (root / "hex-meeting-english.ps1").write_text("param($MeetingDir)\nWrite-Output 'notes deferred engine busy'\nWrite-Output 'PRIVATE_SENTINEL'")
            (root / "hex-meeting-quick-title.ps1").write_text("param($MeetingDir)\nreturn")
            script = root / "run.ps1"
            script.write_text("$ErrorActionPreference='Stop'\n(Get-Item -LiteralPath '" + str(marker) +
                              "').LastWriteTimeUtc = [DateTime]::UtcNow.AddMinutes(-20)\n& '" + str(root / "hex-meeting-watch.ps1") + "'")
            result = subprocess.run(['powershell', '-NoProfile', '-File', str(script)],
                                    capture_output=True, text=True, timeout=20,
                                    creationflags=subprocess.CREATE_NO_WINDOW,
                                    env=os.environ | {'SIDENOTE_DATA_DIR': str(root)})
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(marker.read_text(), "try synthetic failure")
            self.assertIn("notes deferred engine busy", log.read_text(encoding="utf-8-sig"))
            self.assertNotIn("done", log.read_text(encoding="utf-8-sig"))
            self.assertNotIn("PRIVATE_SENTINEL", log.read_text(encoding="utf-8-sig"))

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
                                    env=os.environ | {'SIDENOTE_DATA_DIR': str(root)})
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(list(meeting.iterdir()), [meeting / '.notes-engine.lock'])
            print(result.stdout.strip())


if __name__ == '__main__':
    unittest.main()
