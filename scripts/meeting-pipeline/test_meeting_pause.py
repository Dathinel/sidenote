"""Exercise the scheduled and explicit script entry with made-up folders only."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).with_name('hex-meeting-watch.ps1')

class MeetingPause(unittest.TestCase):
    def test_paused_parts_are_held_and_explicit_joined_folder_is_used(self):
        with tempfile.TemporaryDirectory(prefix='sidenote-pause-script-') as tmp:
            root = Path(tmp)
            scripts = root / 'scripts'
            scripts.mkdir()
            shutil.copy2(SOURCE, scripts / SOURCE.name)
            (scripts / 'hex-pipeline-cli.ps1').write_text(
                "function Get-ScriptsNotesEngine { 'scripts' }\n"
                "function Get-PipelineFailure { 'synthetic pipeline failure' }\n")
            (scripts / 'hex-meeting-english.ps1').write_text(
                "param([string]$MeetingDir,[switch]$NotesOnly,[switch]$TranscriptOnly)\n"
                "if ($TranscriptOnly) { [IO.File]::WriteAllText((Join-Path $MeetingDir 'english-transcript.md'), ('[00:00:01.000] Them (English): Synthetic part '+(Split-Path -Leaf $MeetingDir))); return }\n"
                "if (-not $NotesOnly) { throw 'Joined input must use notes only' }\n"
                "[IO.File]::WriteAllText((Join-Path $MeetingDir 'english-summary.json'), '{\"title\":\"Synthetic notes\"}')\n")
            (scripts / 'hex-meeting-quick-title.ps1').write_text(
                "param([string]$MeetingDir)\n"
                "[IO.File]::WriteAllText((Join-Path $MeetingDir 'quick-title.json'), '{\"title\":\"Synthetic title\"}')\n")
            appdata = root / 'appdata'
            physical = appdata / 'hex/meetings/100-1'
            state = root / 'state'
            joined = state / 'local/meeting-inputs/100-1'
            for folder in (physical, joined):
                folder.mkdir(parents=True)
                (folder / 'manifest.json').write_text(json.dumps(dict(status='complete', duration_ms=60000)))
                (folder / 'transcript.ndjson').write_text('{"start_ms":0,"text":"Only synthetic words."}')
            (joined / 'sidenote-parts.json').write_text(json.dumps([dict(id='100-1',duration_ms=60000),dict(id='200-2',duration_ms=60000)]))
            store = state / 'local/meeting-parts'
            store.mkdir()
            (store / '100-1.json').write_text(json.dumps(dict(id='100-1', parts=[dict(id='100-1')], phase='Paused')))
            local = root / 'localappdata'
            local.mkdir()
            env = os.environ | dict(APPDATA=str(appdata), LOCALAPPDATA=str(local), SIDENOTE_DATA_DIR=str(state))
            command = ['powershell.exe', '-NoProfile', '-NonInteractive', '-File', str(scripts / SOURCE.name)]
            flags = getattr(subprocess, 'CREATE_NO_WINDOW', 0)
            result = subprocess.run(command, env=env, check=True, creationflags=flags, timeout=30, capture_output=True)
            self.assertFalse((physical / 'english-summary.json').exists(),result.stdout.decode())
            self.assertFalse((physical / 'quick-title.json').exists())
            pending = appdata / 'hex/meetings/200-2'
            pending.mkdir()
            (pending / 'manifest.json').write_text(json.dumps(dict(status='complete', created_at_ms=200, duration_ms=60000)))
            (pending / 'transcript.ndjson').write_text('{"start_ms":0,"text":"A pending synthetic part."}')
            (store / '100-1.json').write_text(json.dumps(dict(id='100-1', parts=[dict(id='100-1')], phase='Paused', pending_start_ms=200)))
            subprocess.run(command, env=env, check=True, creationflags=flags, timeout=30, capture_output=True)
            self.assertFalse((pending / 'english-summary.json').exists())
            self.assertFalse((pending / 'quick-title.json').exists())
            subprocess.run(command + ['-MeetingFolders', str(joined)], env=env, check=True,
                           creationflags=flags, timeout=30, capture_output=True)
            self.assertTrue((joined / 'english-summary.json').exists())
            text=(joined / 'english-transcript.md').read_text()
            self.assertLess(text.index('Synthetic part 100-1'),text.index('Synthetic part 200-2'))
            self.assertIn('[00:01:01.000]',text)
            self.assertFalse((physical / 'english-summary.json').exists())

    def test_joined_notes_only_uses_transcript_without_hex_or_audio(self):
        with tempfile.TemporaryDirectory(prefix='sidenote-joined-notes-') as tmp:
            root=Path(tmp)
            scripts=root/'scripts'
            scripts.mkdir()
            source=SOURCE.with_name('hex-meeting-english.ps1')
            shutil.copy2(source,scripts/source.name)
            (scripts/'hex-pipeline-cli.ps1').write_text(r"""
$script:JobContract=[pscustomobject]@{preserve_completed=$true;summary_file='english-summary.json'}
function Get-ScriptsNotesEngine { 'scripts' }
function Enter-NotesEngine($folder) { [IO.File]::Open((Join-Path $folder 'synthetic.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None) }
function Read-HexPrompt($path) { 'Synthetic prompt' }
function Get-PipelineFailure($record) { $record.Exception.Message }
function Invoke-HexCli {
    param($CliArgs,$TimeoutSeconds,$Stage)
    if ($Stage -notin @('render','tasks')) { throw 'Unexpected audio or Hex stage' }
    [IO.File]::AppendAllText($env:SIDENOTE_TEST_TRACE,$Stage+"`n")
}
""")
            (scripts/'hex-meeting-quick-title.ps1').write_text('param($MeetingDir)\n')
            (scripts/'hex-meeting-compare-delayed.ps1').write_text("throw 'Joined meeting must not run Hex comparison'\n")
            (scripts/'hex-meeting-notes.ps1').write_text(r"""
param($InputFile,$OutFile,$MeetingDir,$NotesClaim,$Project,$Model)
Copy-Item -LiteralPath $InputFile -Destination (Join-Path $MeetingDir 'notes-request.txt')
[IO.File]::WriteAllText($OutFile,'{"title":"Synthetic joined notes","context":[],"topics":[],"todo":[],"to_settle":[]}')
""")
            folder=root/'100-1'
            folder.mkdir()
            (folder/'manifest.json').write_text('{"status":"complete","created_at_ms":100}')
            (folder/'sidenote-parts.json').write_text('[]')
            transcript='[00:00:01.000] You (English): First synthetic part.\n[00:01:01.000] Them (English): Second synthetic part.\n'
            (folder/'english-transcript.md').write_text(transcript)
            for name in ('appdata','localappdata','profile'):
                (root/name).mkdir()
            trace=root/'trace.txt'
            env=os.environ|dict(APPDATA=str(root/'appdata'),LOCALAPPDATA=str(root/'localappdata'),
                                USERPROFILE=str(root/'profile'),SIDENOTE_TEST_TRACE=str(trace),SIDENOTE_DATA_DIR=str(root/'state'))
            result=subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-File',str(scripts/source.name),
                                   '-MeetingDir',str(folder),'-NotesOnly'],env=env,capture_output=True,
                                  creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0),timeout=30)
            self.assertEqual(result.returncode,0,result.stderr.decode(errors='replace'))
            self.assertEqual(trace.read_text().splitlines(),['render','tasks'])
            request=(folder/'notes-request.txt').read_text()
            self.assertLess(request.index('First synthetic part'),request.index('Second synthetic part'))
            self.assertEqual((folder/'english-transcript.md').read_text(),transcript)
            self.assertTrue((folder/'english-summary.json').exists())
            self.assertFalse((root/'profile/Documents').exists())

    def test_transcript_only_reuses_a_finished_part_without_notes(self):
        with tempfile.TemporaryDirectory(prefix='sidenote-part-cache-') as tmp:
            root=Path(tmp); scripts=root/'scripts'; scripts.mkdir(); folder=root/'100-1'; folder.mkdir()
            source=SOURCE.with_name('hex-meeting-english.ps1'); shutil.copy2(source,scripts/source.name)
            (scripts/'hex-pipeline-cli.ps1').write_text("function Get-ScriptsNotesEngine { 'scripts' }\nfunction Get-PipelineFailure($record) { $record.Exception.Message }\nfunction Enter-NotesEngine { throw 'A cached part must not start work' }\n")
            text='[00:00:01.000] You (English): A cached synthetic part.\n'
            (folder/'english-transcript.md').write_text(text)
            env=os.environ|dict(APPDATA=str(root/'appdata'),LOCALAPPDATA=str(root/'localappdata'),USERPROFILE=str(root/'profile'),SIDENOTE_DATA_DIR=str(root/'state'))
            result=subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-File',str(scripts/source.name),'-MeetingDir',str(folder),'-TranscriptOnly'],env=env,capture_output=True,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0),timeout=30)
            self.assertEqual(result.returncode,0,result.stderr.decode(errors='replace'))
            self.assertEqual((folder/'english-transcript.md').read_text(),text)
            self.assertFalse((folder/'english-summary.json').exists())

if __name__ == '__main__':
    unittest.main()
