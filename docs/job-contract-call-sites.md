# Meeting job contract spawn/read ledger

Decisions needed: none.

Old: integration/dev28-next (8407e2d075c02a0865b31ee71b2c7a43e9d7ccef). New: delivered core/job-contract stack.
Direct detector sites, script calls and delegated I/O are listed old -> new as line:column; + means added, - removed. This includes unchanged sites and synthetic tests.

## scripts/meeting-pipeline/hex-meeting-english.ps1

- 21:16 -> 21:16: `$notesEngine = Get-ScriptsNotesEngine`
- 38:15 -> 38:15: `$latest = Get-ChildItem -LiteralPath $root -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1`
- 42:8 -> 42:8: `$dir = Get-Item -LiteralPath $MeetingDir`
- 44:15 -> 44:15: `$notesClaim = Enter-NotesEngine $dir.FullName`
- 51:6 -> 53:6: `try { & (Join-Path $PSScriptRoot 'hex-meeting-quick-title.ps1') -MeetingDir $dir.FullName } catch { Write-Host "quick-title failed; detail bytes=$([Text.Encoding]::UTF8.GetByteCount([string]$_))" }`
- 52:13 -> 54:13: `$version = (Get-Content -LiteralPath (Join-Path $env:APPDATA 'hex\current-version') -Raw).Trim()`
- 66:10 -> 68:10: `function Invoke-Native([string]$exe, [string[]]$arguments, [int]$timeoutSeconds = 1800, [string]$Stage = 'prepare') {`
- 68:5 -> 70:5: `Invoke-HexCli -CliArgs (@($exe) + $arguments) -TimeoutSeconds $timeoutSeconds -Stage $Stage`
- 70:10 -> 72:10: `function Run-LLM([string]$instruction, [string]$contents, [string]$tag) {`
- 75:5 -> 77:5: `Invoke-HexCli -Kind translate -InputFile $inputFile -OutputFile $outputFile -Model $Model -TimeoutSeconds $LlmTimeoutSeconds -Stage translate`
- 76:23 -> 78:23: `return [IO.File]::ReadAllText($outputFile, $utf8).Trim()`
- 87:5 -> 89:5: `Invoke-Native $uv.Source ($uvArgs + $prepareArgs)`
- 90:9 -> 92:9: `Invoke-Native $uv.Source ($uvArgs + @('observe','--temp',$temp,'--routing-config',$RoutingConfig,'--lid-exe',$lidExe)) 360 'prepare'`
- 94:9 -> 96:9: `Invoke-Native $uv.Source ($uvArgs + @('plan','--temp',$temp) + $routeArgs) 360 'prepare'`
- 96:29 -> 98:29: `$audioInfo = [IO.File]::ReadAllText((Join-Path $temp 'audio-analysis.json'), $utf8) | ConvertFrom-Json`
- 113:5 -> 115:5: `Invoke-Native $uv.Source ($uvArgs + $transcribeArgs) $asrDeadline 'transcribe'`
- 114:27 -> 116:27: `$asrInfo = [IO.File]::ReadAllText((Join-Path $temp 'asr-timings.json'), $utf8) | ConvertFrom-Json`
- 116:26 -> 118:26: `$chunks = [IO.File]::ReadAllText((Join-Path $temp 'chunks.json'), $utf8) | ConvertFrom-Json`
- 127:35 -> 129:35: `$lidEvidence = [IO.File]::ReadAllText($lidEvidenceFile, $utf8) | ConvertFrom-Json`
- 191:30 -> 193:30: `$mf = [IO.File]::ReadAllText($manifestPath, $utf8) | ConvertFrom-Json`
- 201:68 -> 203:68: `$myNotes = if (Test-Path -LiteralPath $NotesFile) { [IO.File]::ReadAllText($NotesFile, $utf8).Trim() } else { '' }`
- 203:14 -> 205:14: `$rule = (Read-HexPrompt (Join-Path $promptDir 'meeting-translate-prompt.md')) + "'n## Glossary'n$glossary'n'n## Context'n$context'n'nThe meeting entries are in the Input section."`
- 205:35 -> 207:35: `$transcript = ([IO.File]::ReadAllText((Join-Path $dir.FullName 'english-transcript.md'), $utf8) -replace '^\s*# Full English transcript\s*', '').Trim()`
- 210:27 -> 212:27: `$transcripts.Add((Run-LLM $rule $batches[$i] "batch-$i"))`
- 219:80 -> 221:80: `if (Test-Path -LiteralPath $qt) { $workingTitle = ([string]([IO.File]::ReadAllText($qt, $utf8) | ConvertFrom-Json).title).Trim() }`
- 223:19 -> 225:19: `$notesRule = (Read-HexPrompt (Join-Path $promptDir 'meeting-notes-prompt.md')) + "'n## Glossary'n$glossary'n'nThe Context, My notes and Transcript are in the Input section.$workingTitleRule"`
- 229:4 -> 231:4: `& (Join-Path $promptDir 'hex-meeting-notes.ps1') -InputFile $notesIn -OutFile $notesOut -MeetingDir $dir.FullName -NotesClaim $notesClaim -Project $Project -Model $NotesModel`
- 230:25 -> 232:25: `$notes = [IO.File]::ReadAllText($notesOut, $utf8).Trim()`
- 268:9 -> 270:9: `Invoke-HexCli -CliArgs @('uv','run','--quiet','--no-project','python','-I',(Join-Path $promptDir 'hex-meeting-render.py'),$dir.FullName) -TimeoutSeconds 120 -Stage render`
- 273:9 -> 275:9: `Invoke-HexCli -CliArgs @('uv','run','--quiet','--no-project','python','-I',(Join-Path $promptDir 'hex-meeting-publish.py'),$dir.FullName) -TimeoutSeconds 120 -Stage publish`
- 279:9 -> 281:9: `Invoke-HexCli -CliArgs @('uv','run','--quiet','--no-project','python','-I',(Join-Path $promptDir 'hex-meeting-tasks.py'),'--force',$dir.FullName) -TimeoutSeconds 360 -Stage tasks`
- 285:8 -> 287:8: `& (Join-Path $promptDir 'hex-meeting-compare-delayed.ps1') -MeetingDir $dir.FullName`

## scripts/meeting-pipeline/hex-meeting-notes.ps1

- 16:16 -> 16:16: `$notesEngine = Get-ScriptsNotesEngine`
- 34:23 -> 34:23: `$ownedClaim = Enter-NotesEngine $MeetingDir`
- 38:20 -> 38:20: `$text = [IO.File]::ReadAllText($InputFile, $utf8)`
- 61:9 -> 61:9: `Invoke-HexCli -CliArgs @('git','-C',$RepoDir,'fetch','-q','origin') -OutputFile $rosterFile -TimeoutSeconds 60 -Stage notes`
- 62:9 -> 62:9: `Invoke-HexCli -CliArgs @('git','-C',$RepoDir,'show','origin/master:docs/reference/people-and-roles.md') -OutputFile $rosterFile -TimeoutSeconds 30 -Stage notes`
- 63:30 -> 63:30: `$roster = [IO.File]::ReadAllText($rosterFile, $utf8)`
- 77:5 -> 77:5: `Invoke-HexCli -Kind claude -InputFile $promptFile -OutputFile $responseFile -Model $Model -TimeoutSeconds $TimeoutSeconds -Stage notes`
- 78:23 -> 78:23: `$out = [IO.File]::ReadAllText($responseFile, $utf8)`

## scripts/meeting-pipeline/hex-pipeline-cli.ps1

- + -> 1:34: `$script:JobContract = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'job-contract.json')) | ConvertFrom-Json`
- 24:10 -> 28:10: `function Invoke-HexCli {`
- 47:12 -> 52:12: `& $python.Source -I $runner --request $request --diagnostic $diagnostic 2>&1 | Out-Null`
- 52:75 -> 57:75: `if ((Test-Path -LiteralPath $diagnostic -PathType Leaf) -and (Get-Item -LiteralPath $diagnostic).Length -le 1024) {`
- 54:40 -> 59:40: `$data = [IO.File]::ReadAllText($diagnostic) | ConvertFrom-Json`
- 73:10 -> 78:10: `function Read-HexPrompt([string]$Path) {`
- 74:24 -> 79:24: `$text = [IO.File]::ReadAllText($Path, (New-Object Text.UTF8Encoding($false)))`
- 83:10 -> 88:10: `function Enter-NotesEngine([string]$MeetingDir) {`
- 86:25 -> 91:25: `return [IO.File]::Open((Join-Path $MeetingDir '.notes-engine.lock'), => return [IO.File]::Open((Join-Path $MeetingDir $script:JobContract.lock_file),`
- 94:10 -> 99:10: `function Get-ScriptsNotesEngine {`
- 103:39 -> 108:39: `$child = [Diagnostics.Process]::Start($info)`
- 104:41 -> 109:41: `$output = $child.StandardOutput.ReadToEndAsync()`
- 105:40 -> 110:40: `$errors = $child.StandardError.ReadToEndAsync()`

## scripts/meeting-pipeline/hex-pipeline-run.py

- + -> 14:69: `CONTRACT = json.loads(Path(__file__).with_name("job-contract.json").read_text(encoding="utf-8"))`
- 59:5 -> 62:5: `def execute(args, cwd, timeout, input_text=None, env=None):`
- 62:19 -> 65:19: `process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.PIPE,`
- 73:31 -> 76:31: `stopped = subprocess.run(["taskkill.exe", "/PID", str(process.pid), "/T", "/F"],`
- 92:5 -> 95:5: `def claude(text, model, timeout):`
- 94:16 -> 97:16: `return execute([shutil.which("claude") or "claude", "-p", "--model", model,`
- 99:5 -> 102:5: `def translate(text, model, timeout):`
- 115:18 -> 118:18: `output = execute([str(exe), "run", "--standalone", "--agent", agent,`
- 150:31 -> 153:31: `request = json.loads(path.read_text(encoding="utf-8-sig"))`
- 159:40 -> 162:40: `text = Path(request["input_file"]).read_text(encoding="utf-8-sig") if request.get("input_file") else None`
- 162:18 -> 165:18: `output = translate(text, request["model"], timeout)`
- 164:18 -> 167:18: `output = claude(text, request["model"], timeout)`
- 166:18 -> 169:18: `output = execute(request["args"], request.get("cwd") or None, timeout, text)`

## scripts/meeting-pipeline/test_job_contract.py

- + -> 8:13: `spec.loader.exec_module(runner)`

## src/core/live.rs

- 30:8 -> 30:8: `pub fn notes_problem(folder: &Path) -> Option<&'static str> {`
- + -> 33:40: `} else if let Some(cause) = super::meeting_job::script_failure(folder) {`
- 114:22 -> 118:22: `let raw = super::file_reads::read(&path, ROLLING_MAX_BYTES)?;`
- 139:30 -> 143:30: `let Ok(raw) = super::file_reads::read(&folder.join("manifest.json"), 1024 * 1024) else {`
- 166:28 -> 170:28: `let Ok(bytes) = super::file_reads::read(&path, NOTES_MAX_BYTES) else {`
- 347:20 -> 351:20: `assert_eq!(notes_problem(&folder), Some(NOTES_FAILED));`
- + -> 358:27: `let message = notes_problem(&folder).unwrap();`
- 349:20 -> 363:20: `assert_eq!(notes_problem(&folder), None);`
- 362:20 -> 376:20: `assert_eq!(fs::read_to_string(&recovery).unwrap(), "unsaved text");`
- 451:24 -> 465:24: `assert_eq!(fs::read_to_string(&recovery).unwrap(), "meeting B draft");`

## src/core/meeting_job.rs

- + -> 110:42: `match super::meeting_router::inputs::Snapshot::read_file(`
- + -> 155:30: `serde_json::from_str(include_str!(`
- + -> 165:12: `super::store_io::lock(&folder.join(&contract().lock_file))`
- + -> 186:41: `let mut command = std::process::Command::new("powershell.exe");`
- + -> 195:27: `assert_eq!(command.status().unwrap().code(), Some(10));`
- + -> 197:27: `assert_eq!(command.status().unwrap().code(), Some(0));`
- + -> 276:18: `scope.spawn(|| {`

## src/core/notes/mod.rs

- 142:12 -> 158:12: `super::Config::load()?`
- 175:5 -> 191:5: `queue::read(&super::state_dir(), meeting_id)`
- 228:8 -> 244:8: `pub fn scripts_policy() -> &'static str {`
- 229:18 -> 245:18: `match super::Config::load_existing(&super::state_dir()) {`

## src/core/notes/queue.rs

- 87:11 -> 91:11: `match store_io::read_bounded(&job_path(root, id), file_reads::JOURNAL_MAX) {`
- 120:47 -> 128:47: `let bytes = core::meeting_router::inputs::Snapshot::read_file(`
- 162:17 -> 170:17: `let _lock = store_io::lock(&jobs(root).join(format!("{id}.lock")))?;`
- + -> 201:8: `if meeting_job::preserve_summary(&folder, sorting) {`
- 238:41 -> 246:41: `match core::meeting_router::inputs::Snapshot::read_file(path, cap) {`
- 248:11 -> 256:11: `match store_io::read_bounded(&path, file_reads::JOURNAL_MAX) {`
- 291:24 -> 299:24: `let _claim = match store_io::lock(&folder.join(".notes-engine.lock")) { => let _claim = match meeting_job::claim(&folder) {`
- + -> 323:8: `if meeting_job::preserve_summary(&folder, sort_only) {`
- 365:13 -> 374:13: `transcript::read(&folder)?`
- 370:9 -> 379:9: `transcript::write(&folder)?`
- 496:27 -> 506:27: `let title = core::read_quick_title(&folder)`
- 626:25 -> 639:25: `let _worker = match store_io::lock(&jobs(root).join("worker.lock")) {`
- 721:25 -> 734:25: `let _job_lock = store_io::lock(&jobs(root).join(format!("{id}.lock")))?;`
- 842:9 -> 855:9: `.spawn()`
- 850:25 -> 863:25: `let _worker = match store_io::lock(&jobs(&root).join("worker.lock")) {`
- 939:12 -> 953:12: `},Some(include_str!("tasks_redo_tail.txt"))) {`

## src/core/notes/tests.rs

- 40:22 -> 40:22: `crate::core::file_reads::nofollow(&path).is_err(),`
- 110:39 -> 110:39: `let config: crate::core::Config = store_io::read(&root.join("config.json"))?;`
- 268:33 -> 268:33: `serde_json::from_slice(&fs::read(folder.join("manifest.json")).unwrap()).unwrap();`
- 357:18 -> 357:18: `let before = fs::read(&path).unwrap();`
- 364:16 -> 364:16: `assert_eq!(fs::read(&path).unwrap(), before);`
- 367:16 -> 367:16: `assert_eq!(fs::read(&path).unwrap(), before);`
- 371:9 -> 371:9: `queue::read(&root.0, ID).unwrap().unwrap().state,`
- 387:15 -> 387:15: `let job = queue::read(&root.0, ID).unwrap().unwrap();`
- 418:5 -> 418:5: `queue::read(root, ID).unwrap().unwrap().state`
- 480:9 -> 480:9: `fs::read_to_string(folder.join("english-transcript.md")).unwrap(),`
- 534:33 -> 534:33: `serde_json::from_slice(&fs::read(folder.join("english-summary.json")).unwrap()).unwrap();`
- 540:33 -> 540:33: `serde_json::from_slice(&fs::read(folder.join("tasks.json")).unwrap()).unwrap();`
- 611:9 -> 611:9: `queue::read(&root.0, ID).unwrap().unwrap().instructions,`
- 627:9 -> 627:9: `fs::read_to_string(folder.join("english-transcript.md"))`
- 643:9 -> 643:9: `fs::read_to_string(folder.join("english-transcript.md"))`
- 656:13 -> 656:13: `assert!(queue::read(&root.0, ID).unwrap().is_none());`
- 658:9 -> 658:9: `fs::read(folder.join("english-summary.json")).unwrap(),`
- 668:16 -> 668:16: `let lock = store_io::lock(&root.0.join("local/notes-jobs/worker.lock")).unwrap();`
- 682:17 -> 682:17: `let _lock = store_io::lock(&root.0.join(format!("local/notes-jobs/{ID}.lock"))).unwrap();`
- 697:23 -> 697:23: `let mut job = queue::read(&root.0, ID).unwrap().unwrap();`
- 730:10 -> 730:10: `drop(store_io::lock(&path).expect("idle worker released its lock"));`
- 731:19 -> 731:19: `let mut job = queue::read(&root.0, ID).unwrap().unwrap();`
- 737:9 -> 737:9: `queue::read(&root.0, ID).unwrap().unwrap().next_try_at,`
- 800:18 -> 800:18: `assert!(!fs::read_to_string(path).unwrap().contains(PRIVATE));`
- 825:17 -> 825:17: `assert!(queue::read(&root.0, ID).unwrap().is_none());`
- 866:33 -> 866:33: `serde_json::from_slice(&fs::read(root.0.join("config.json")).unwrap()).unwrap();`
- 890:9 -> 890:9: `fs::read_to_string(folder.join("english-summary.json")).unwrap(),`
- 931:16 -> 931:16: `let lock = store_io::lock(&root.0.join("local/config.lock")).unwrap();`
- 967:9 -> 967:9: `fs::read(quarantined[0].path()).unwrap(),`
- 971:10 -> 971:10: `!fs::read_to_string(root.0.join("sidenote.log"))`
- 1027:9 -> 1027:9: `queue::read(&root.0, new_id).unwrap().unwrap().state,`
- 1112:9 -> 1112:9: `fs::read_to_string(folder.join("notes.md"))`
- 1137:19 -> 1137:19: `let mut job = queue::read(&root.0, ID).unwrap().unwrap();`
- + -> 1152:9: `queue::read(&root.0, ID).unwrap().unwrap().stage,`
- 1151:19 -> 1155:19: `let summary = fs::read(folder.join("english-summary.json")).unwrap();`
- + -> 1156:27: `let mut interrupted = queue::read(&root.0, ID).unwrap().unwrap();`
- 1156:9 -> 1167:9: `fs::read(folder.join("english-summary.json")).unwrap(),`
- 1189:9 -> 1200:9: `fs::read(folder.join("english-summary.json")).unwrap(),`
- 1214:33 -> 1225:33: `serde_json::from_slice(&fs::read(folder.join("english-summary.json")).unwrap()).unwrap();`
- 1248:9 -> 1259:9: `fs::read_to_string(folder.join("english-summary.json")).unwrap(),`
- 1278:9 -> 1289:9: `fs::read_to_string(folder.join("english-transcript.md")).unwrap(),`
- 1358:9 -> 1369:9: `queue::read(&root.0, ID).unwrap().unwrap().instructions,`
- 1367:17 -> 1378:17: `let saved = fs::read(folder.join("english-summary.json")).unwrap();`
- 1377:9 -> 1388:9: `fs::read(folder.join("english-summary.json")).unwrap(),`
- 1401:13 -> 1412:13: `assert!(queue::read(&root.0, ID).unwrap().is_none());`
- 1413:13 -> 1424:13: `assert!(queue::read(&root.0, ID).unwrap().is_none());`
- 1442:18 -> 1453:18: `let before = fs::read(root.0.join("local/notes-jobs").join(format!("{ID}.json"))).unwrap();`
- 1446:9 -> 1457:9: `fs::read(root.0.join("local/notes-jobs").join(format!("{ID}.json"))).unwrap(),`
- 1461:20 -> 1472:20: `let original = include_str!("../../../tests/fixtures/hex-labs/original.ndjson");`
- 1465:9 -> 1476:9: `transcript::read(&folder)`
- 1472:9 -> 1483:9: `transcript::read(&folder)`
- 1499:19 -> 1510:19: `let job = queue::read(&root.0, ID).unwrap().unwrap();`
- 1503:14 -> 1514:14: `!fs::read_to_string(root.0.join("sidenote.log"))`
- 1544:19 -> 1555:19: `let mut job = queue::read(&root.0, ID).unwrap().unwrap();`
- 1556:19 -> 1567:19: `let mut job = queue::read(&root.0, ID).unwrap().unwrap();`
- 1571:15 -> 1582:15: `let job = queue::read(&root.0, ID).unwrap().unwrap();`
- 1591:18 -> 1602:18: `let before = fs::read(&path).unwrap();`
- 1598:16 -> 1609:16: `assert_eq!(fs::read(&path).unwrap(), before);`
- 1600:18 -> 1611:18: `let queued = fs::read(&path).unwrap();`
- 1601:15 -> 1612:15: `let job = queue::read(&root.0, ID).unwrap().unwrap();`
- 1607:16 -> 1618:16: `assert_eq!(fs::read(&path).unwrap(), queued);`
- 1629:15 -> 1640:15: `let job = queue::read(&root.0, ID).unwrap().unwrap();`
- 1648:17 -> 1659:30: `let claim = store_io::lock(&folder.join(".notes-engine.lock")).unwrap(); => let claim = crate::core::meeting_job::claim(&folder).unwrap();`
- 1652:16 -> 1663:16: `assert_eq!(queue::read(&root.0, ID).unwrap().unwrap().attempts, 0);`
- + -> 1677:62: `let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();`
- + -> 1681:9: `queue::read(&root.0, ID).unwrap().unwrap().stage,`
- + -> 1686:15: `let job = queue::read(&root.0, ID).unwrap().unwrap();`

## src/meeting_icon.rs

- 67:24 -> 68:24: `match core::notes::scripts_policy() {`
- 76:18 -> 73:32: `|| core::notes::status(id).is_some_and(|s| { => if let Some(cause) = core::meeting_job::script_failure(&core::meetings_dir().join(id))`
- + -> 74:27: `.or_else(|| core::notes::status(id).and_then(|s| s.failure_cause()))`
- 89:20 -> 78:20: `let deferred = store_io::read::<String>(`
- 97:19 -> 86:19: `core::notes::status(id),`
- 102:30 -> 91:30: `} else if matches!(core::notes::status(id), Some(JobState::Queued))`
- 103:28 -> 92:28: `|| (matches!(core::notes::status(id), Some(JobState::Running))`
- 104:16 -> 93:16: `&& claim_busy(&core::state_dir().join("local/notes-jobs/worker.lock")))`
- 105:12 -> 94:12: `|| claim_busy(&core::meetings_dir().join(id).join(".notes-engine.lock")) => || claim_busy(`
- 106:12 -> 99:12: `|| claim_busy(`
- 117:4 -> 110:4: `fn claim_busy(path: &std::path::Path) -> bool {`
- 119:12 -> 112:12: `&& store_io::lock(path).is_err_and(|e| {`
- 215:5 -> 208:5: `file_reads::text(&path, file_reads::SUMMARY_MAX)`
- 223:5 -> 216:5: `store_io::read::<String>(&seen_path(id))`
- 267:9 -> 260:9: `.spawn(move || run(isolated, replay))`
- 276:9 -> 269:9: `store_io::read(&pending_path()).unwrap_or_default();`
- 305:17 -> 298:17: `file_reads::text(`
- 314:23 -> 307:23: `engine.status().ok().and_then(|status| {`
- 441:21 -> 434:21: `store_io::read::<i64>(&dismissed_path(id)).ok(),`

## src/meeting_icon/native/window.rs

- 219:28 -> 219:28: `let fraction = store_io::read::<Option<f32>>(`

## src/pipeline_install.ps1

- 21:10 -> 21:10: `if ((Get-Item -LiteralPath $manifestPath).Length -gt 8192 -or`
- 23:28 -> 23:28: `$manifest = [IO.File]::ReadAllText($manifestPath) | ConvertFrom-Json`
- 59:30 -> 59:30: `$payload = [Console]::In.ReadToEnd() | ConvertFrom-Json`
- 88:14 -> 88:14: `if ((Get-Item -LiteralPath $pointerPath).Length -gt 2048) { throw 'Invalid pointer' }`
- 89:31 -> 89:31: `$pointer = [IO.File]::ReadAllText($pointerPath) | ConvertFrom-Json`
- 100:18 -> 100:18: `if ((Get-Item -LiteralPath $source).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Invalid legacy source' }`
- 120:46 -> 120:46: `if ([IO.File]::Exists($entryPath) -and ((Get-Item -LiteralPath $entryPath).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid entry' }`

## src/updates_windows.rs

- 153:26 -> 153:26: `let mut source = File::open(self.0.join(&asset.FileName))?;`
- 189:20 -> 189:20: `let mut file = File::open(cache_path(asset)?).map_err(|_| "Download the update again.")?;`
- 241:16 -> 241:16: `std::thread::spawn(move || {`
- 292:5 -> 292:5: `fs::read(directory.join("updates.json"))`
- 382:39 -> 382:39: `let worker = scope.spawn(|| mgr.download_updates(&update, Some(tx)));`
- 453:46 -> 453:46: `match crate::hex_engine::HexEngine::new().status() {`
- 608:21 -> 608:21: `let mut child = Command::new("powershell.exe")`
- 626:9 -> 626:9: `.spawn()`
- 712:21 -> 712:21: `let Ok(bytes) = fs::read(&path) else {`
- 723:12 -> 723:12: `if fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) {`
- 929:8 -> 929:8: `if fs::read(marker).map_err(|_| PIPELINE_INSTALL_ERROR)? != VERSION.as_bytes() {`
- 943:13 -> 943:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-compare-delayed.ps1"),`
- 947:13 -> 947:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-compare-granola.py"),`
- 951:13 -> 951:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-english.ps1"),`
- 955:13 -> 955:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-notes.ps1"),`
- 959:13 -> 959:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-publish.py"),`
- 963:13 -> 963:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-quick-title.ps1"),`
- 967:13 -> 967:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-render.py"),`
- 971:13 -> 971:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-tasks.py"),`
- 975:13 -> 975:13: `include_str!("../scripts/meeting-pipeline/hex-meeting-watch.ps1"),`
- 979:13 -> 979:13: `include_str!("../scripts/meeting-pipeline/hex-pipeline-cli.ps1"),`
- + -> 983:13: `include_str!("../scripts/meeting-pipeline/job-contract.json"),`
- 983:13 -> 987:13: `include_str!("../scripts/meeting-pipeline/hex-pipeline-run.py"),`
- 987:13 -> 991:13: `include_str!("../scripts/meeting-pipeline/meeting-audio.py"),`
- 991:13 -> 995:13: `include_str!("../scripts/meeting-pipeline/meeting-notes-prompt.md"),`
- 995:13 -> 999:13: `include_str!("../scripts/meeting-pipeline/meeting-translate-prompt.md"),`
- 1008:17 -> 1012:17: `let entry = include_str!("pipeline_entry.ps1");`
- 1014:21 -> 1018:21: `let mut child = Command::new("powershell.exe")`
- 1019:13 -> 1023:13: `include_str!("pipeline_install.ps1"),`
- 1025:9 -> 1029:9: `.spawn()`
- 1128:35 -> 1132:35: `let verified: VelopackAsset = fs::read(local("update-ready.json"))`
- 1298:20 -> 1302:20: `assert_eq!(fs::read(&marker).unwrap(), VERSION.as_bytes());`
- 1437:33 -> 1441:33: `let server = std::thread::spawn(move || {`
- 1559:26 -> 1563:26: `let status = Command::new("powershell.exe")`
- 1580:17 -> 1584:17: `.status()`

Total: 233 mapped sites across 14 changed source files.

The new meeting-folder read uses Snapshot::read_file with an 8192-byte cap; adapters also read bundled job-contract.json.
New process calls belong to the synthetic Windows interoperability test. Production queue and script launches retain their boundaries.
