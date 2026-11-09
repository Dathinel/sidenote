param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    # Another output folder (for example a before set), and an optional list of shot names.
    [string]$Out = "",
    [string[]]$Only = @(),
    # Themes to render; dark first is enough for a quick look.
    [string[]]$Themes = @('dark', 'light')
)
# Renders the UI headlessly, dark and light, into artifacts/ui. Each scene gets its own empty
# state folder, so ticks, task edits, project renames and Ask threads never touch the real
# ones. The Ask scene starts from
# tools/ui-fixtures/ask-answer.json, an invented Northwind answer; the
# home-ask scenes start from tools/ui-fixtures/ask-all-answer.json, also invented.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$out = if ($Out) { $Out } else { "$root/artifacts/ui" }
$meeting = '1800000000000-1'
New-Item -ItemType Directory -Force -Path $out | Out-Null

# name, scene, size, script (or $null), extra args
$shots = @(
    @('updates-manual', 'updates-manual', '1320x1000', $null, @()),
    @('updates-manual-narrow', 'updates-manual', '560x1100', $null, @()),
    @('updates-preparing', 'updates-preparing', '1320x1000', $null, @()),
    @('updates-preparing-narrow', 'updates-preparing', '560x1100', $null, @()),
    @('updates-stopping', 'updates-stopping', '1320x1000', $null, @()),
    @('updates-stopping-narrow', 'updates-stopping', '560x1100', $null, @()),
    @('updates-later', 'updates-ready', '560x1100', 'updates-later.json', @('--at', 'mark:end')),
    @('updates-notes', 'updates-available', '560x1100', 'updates-notes.json', @('--at', 'mark:end')),
    @('updates-none', 'updates-none', '1320x1000', $null, @()),
    @('updates-none-narrow', 'updates-none', '560x1100', $null, @()),
    @('updates-available', 'updates-available', '1320x1000', $null, @()),
    @('updates-available-narrow', 'updates-available', '560x1100', $null, @()),
    @('updates-downloading', 'updates-downloading', '1320x1000', $null, @()),
    @('updates-downloading-narrow', 'updates-downloading', '560x1100', $null, @()),
    @('updates-ready', 'updates-ready', '1320x1000', $null, @()),
    @('updates-ready-narrow', 'updates-ready', '560x1100', $null, @()),
    @('updates-error', 'updates-error', '1320x1000', $null, @()),
    @('updates-error-narrow', 'updates-error', '560x1100', $null, @()),
    @('routing-settings', 'routing-settings', '1320x1000', $null, @()),
    @('routing-settings-narrow', 'routing-settings', '560x1100', $null, @()),
    @('routing-original', 'routing-original', '1320x900', $null, @()),
    @('routing-original-narrow', 'routing-original', '560x1100', $null, @()),
    @('audio-project-meeting', 'audio-project-meeting', '1320x900', $null, @()),
    @('audio-project-existing', 'audio-project-existing', '1320x800', $null, @()),
    @('audio-project-new', 'audio-project-new', '1320x800', $null, @()),
    @('audio-project-created', 'audio-project-created', '1320x800', $null, @()),
    @('audio-project-dismissed', 'audio-project-dismissed', '1320x800', $null, @()),
    @('models-empty', 'models-empty', '1320x1000', $null, @()),
    @('models-downloading', 'models-downloading', '1320x1000', $null, @()),
    @('models-installed', 'models-installed', '1320x1000', $null, @()),
    @('models-installed-narrow', 'models-installed', '560x1100', $null, @()),
    @('models-checksum-error', 'models-checksum-error', '1320x1000', $null, @()),
    @('audio-settings', 'hex-meetings', '1320x900', $null, @()),
    @('features-off-sidebar', 'features-off-sidebar', '1320x800', $null, @()),
    @('features-on-sidebar', 'home-open', '1320x800', $null, @()),
    @('features-settings', 'features-settings', '1320x900', $null, @()),
    @('features-off-home', 'features-off-home', '1320x800', $null, @()),
    @('features-off-meeting', 'features-off-meeting', '1320x900', $null, @()),
    @('features-tasks-only', 'features-tasks-only', '1320x900', $null, @()),
    @('features-toggled', 'features-settings', '1320x900', 'features-toggle.json', @('--at', 'mark:end')),

    @('list',       'list',       '1320x800', $null,           @()),
    @('search',     'search',     '1320x800', $null,           @()),
    @('meeting',    'meeting',    '1320x800', $null,           @()),
    @('transcript', 'transcript', '1320x800', $null,           @('--at', '0.6')),
    @('collapsed',  'collapsed',  '1320x800', $null,           @()),
    @('decisions',  'decisions',  '1320x800', $null,           @()),
    @('project-menu', 'project-menu', '1320x800', $null,       @()),
    @('project-accepted', 'project-accepted', '1320x800', $null, @()),
    @('discussion', 'discussion', '1320x800', $null,           @('--at', '0.6')),
    @('todo',       'todo',       '1320x800', $null,           @()),
    @('jump',       'jump',       '1320x800', $null,           @('--at', '0.6')),
    @('ask',        'ask',        '1320x800', $null,           @()),
    @('ask-hidden', 'ask',        '1320x800', 'ask-hide.json', @('--at', 'mark:end')),
    @('ask-running','ask-running','1320x800', $null,           @()),
    @('empty',      'empty',      '1320x800', $null,           @()),
    # The home page's Ask, across all meetings. home-ask starts from tools/ui-fixtures/ask-all-answer.json,
    # an invented Northwind answer; home-ask-cite taps its first citation.
    @('home-ask',         'home-ask',         '1320x800', $null,                @()),
    @('home-ask-hidden',  'home-ask',         '1320x800', 'ask-hide.json',      @('--at', 'mark:end')),
    @('home-ask-running', 'home-ask-running', '1320x800', $null,                @()),
    @('home-ask-cite',    'home-ask',         '1320x800', 'ask-all-cite.json',  @('--at', 'mark:end')),
    @('home-ask-back',    'home-ask',         '1320x800', 'ask-all-back.json',  @('--at', 'mark:end')),
    @('home-ask-narrow',  'home-ask',         '560x900',  $null,                @()),
    @('keys',       'list',       '1320x800', 'keys.json',     @('--at', 'mark:end')),
    @('runner',     'meeting',    '1320x800', 'runner.json',   @('--at', 'mark:end')),
    @('narrow',     'meeting',    '560x900',  $null,           @()),
    @('narrow-list','meeting',    '560x900',  'narrow-list.json', @('--at', 'mark:end')),
    # The Tasks view. The Mine, tick, jump and owner shots press real controls through a script.
    @('tasks',          'tasks',         '1320x800', $null,                @()),
    @('tasks-meeting',  'tasks-meeting', '1320x800', $null,                @()),
    @('tasks-due',      'tasks-due',     '1320x800', $null,                @()),
    @('tasks-mine',     'tasks',         '1320x800', 'tasks-mine.json',    @('--at', 'mark:end')),
    @('tasks-menu',     'tasks-menu',    '1320x800', $null,                @()),
    @('tasks-edit',     'tasks-edit',    '1320x800', $null,                @()),
    @('tasks-renamed',  'tasks-groups',       '1320x800', 'tasks-rename.json',  @('--at', 'mark:end')),
    @('tasks-tick',     'tasks',         '1320x800', 'tasks-tick.json',    @('--at', 'mark:end')),
    @('tasks-owner',    'tasks-edit-owner', '1320x800', 'tasks-owner.json', @('--at', 'mark:end')),
    @('tasks-due-set',  'tasks-edit',    '1320x800', 'tasks-due-type.json', @('--at', 'mark:end')),
    @('tasks-due-bad',  'tasks-edit',    '1320x800', 'tasks-due-bad.json', @('--at', 'mark:end')),
    @('tasks-sync',     'tasks',         '1320x800', 'tasks-sync.json',    @('--at', 'mark:end')),
    @('tasks-sync-back', 'tasks',        '1320x800', 'tasks-sync-back.json', @('--at', 'mark:end')),
    # Taller, so the panel's Read on link is on screen when the script presses it.
    @('tasks-jump',     'tasks',         '1320x1000', 'tasks-jump.json',   @('--at', 'mark:end')),
    @('tasks-narrow',   'tasks',         '560x900',  $null,                @()),
    # Focus, rows and adding by hand.
    @('tasks-expanded', 'tasks-expanded', '1320x800', $null,               @()),
    # The right-hand panel: a task open, grouped by project, narrow, and the group menu.
    @('tasks-panel',    'tasks-panel',   '1320x800', $null,                @()),
    @('tasks-panel-project', 'tasks-panel-project', '1320x800', $null,     @()),
    @('tasks-panel-click', 'tasks',      '1320x800', 'tasks-open.json',    @('--at', 'mark:end')),
    @('tasks-panel-narrow', 'tasks-panel', '560x900', $null,               @()),
    @('tasks-group-project', 'tasks-group-project', '1320x800', $null,     @()),
    @('tasks-group-menu', 'tasks-group-menu', '1320x800', $null,           @()),
    # The cleared view: tasks-settings.json holds a cutoff, so older meetings' tasks are left off.
    @('tasks-cleared',      'tasks-cleared',      '1320x800', $null,       @()),
    @('tasks-cleared-menu', 'tasks-cleared-menu', '1320x800', $null,       @()),
    @('tasks-keys-panel', 'tasks',       '1320x800', 'tasks-keys-panel.json', @('--at', 'mark:end')),
    @('tasks-quick-due', 'tasks-quick-due', '1320x800', $null,             @()),
    @('tasks-quick-due-set', 'tasks-quick-due', '1320x800', 'tasks-quick-due-pick.json', @('--at', 'mark:end')),
    @('tasks-collapsed', 'tasks-collapsed', '1320x800', $null,             @()),
    @('tasks-done-open', 'tasks-done-open', '1320x800', $null,             @()),
    @('tasks-add',      'tasks-add',     '1320x800', 'tasks-add-type.json', @('--at', 'mark:end')),
    @('tasks-added',    'tasks-add',     '1320x800', 'tasks-add-save.json', @('--at', 'mark:end')),
    @('tasks-keys',     'tasks',         '1320x800', 'tasks-keys.json',    @('--at', 'mark:end')),
    # The Daily check page, against synthetic reports that tools/prepare_check_fixture.py
    # turns into a schema 2 report with invented v2 fields. check-v1* show the old format.
    @('check',          'check',         '1320x800', $null,                @()),
    @('check-panel',    'check-panel',   '1320x800', $null,                @()),
    @('check-applied',  'check-applied', '1320x800', $null,                @()),
    @('check-snooze',   'check-snooze',  '1320x800', $null,                @()),
    @('check-click',    'check',         '1320x800', 'check-click.json',   @('--at', 'mark:end')),
    @('check-keys',     'check',         '1320x800', 'check-keys.json',    @('--at', 'mark:end')),
    @('check-dismiss',  'check-panel',   '1320x800', 'check-dismiss.json', @('--at', 'mark:end')),
    @('check-pick-date', 'check-snooze-pick',  '1320x800', 'check-pick-date.json', @('--at', 'mark:end')),
    @('check-details',  'check-details', '1320x800', $null,                @()),
    @('check-changed',  'check-changed', '1320x800', $null,                @()),
    @('check-story',    'check-story',   '1320x800', $null,                @()),
    @('check-history',  'check-history', '1320x800', $null,                @()),
    @('check-picker',   'check-picker',  '1320x800', $null,                @()),
    @('check-cite',     'check-panel',   '1320x800', 'check-cite.json',    @('--at', 'mark:end')),
    @('check-back',     'check-panel',   '1320x800', 'check-back.json',    @('--at', 'mark:end')),
    @('check-empty',    'check',         '1320x800', $null,                @()),
    @('check-home-card', 'list',         '1320x800', $null,                @()),
    @('check-narrow',   'check',         '560x900',  $null,                @()),
    @('check-panel-narrow', 'check-panel', '560x900', $null,               @()),
    @('check-story-narrow', 'check-story', '560x900', $null,               @()),
    @('check-history-narrow', 'check-history', '560x900', $null,           @()),
    @('check-home-card-narrow', 'list',  '560x900',  $null,                @()),
    @('check-v1',       'check',         '1320x800', $null,                @()),
    @('check-v1-panel', 'check-panel',   '1320x800', $null,                @()),
    # The Memory page: migrated records, the review list, a record panel and the migration banner.
    @('memory',         'memory',        '1320x800', $null,                @()),
    @('memory-panel',   'memory-panel',  '1320x800', $null,                @()),
    @('memory-review',  'memory-review', '1320x800', $null,                @()),
    @('memory-retired', 'memory-retired', '1320x800', $null,               @()),
    @('memory-forget',  'memory-panel',  '1320x800', 'memory-forget.json', @('--at', 'mark:end')),
    @('memory-edit',    'memory-panel',  '1320x800', 'memory-edit.json',   @('--at', 'mark:end')),
    @('memory-confirm', 'memory-panel',  '1320x800', 'memory-confirm.json', @('--at', 'mark:end')),
    @('memory-accept-proposal', 'memory-review', '1320x800', 'memory-accept-proposal.json', @('--at', 'mark:end')),
    @('memory-migration', 'memory',      '1320x800', $null,                @()),
    @('memory-skim',    'memory-skim',   '1320x800', $null,                @()),
    @('memory-migrated', 'memory',       '1320x800', 'memory-accept.json', @('--at', 'mark:end')),
    @('memory-narrow',  'memory',        '560x900',  $null,                @()),
    @('memory-panel-narrow', 'memory-panel', '560x900', $null,             @()),
    @('memory-review-narrow', 'memory-review', '560x900', $null,           @()),
    @('memory-migration-narrow', 'memory', '560x900', $null,               @()),
    # The Hex page and the top bar controls. tools/ui-fixtures/hex.json stands in for the engine, so
    # no render calls the live Hex. At 820 px the window is not yet narrow, so the full controls
    # share the top bar with the other pills (the tightest case); 560 px shows the strip instead.
    @('hex',                    'hex',                '1320x800', $null,   @()),
    @('hex-narrow',             'hex',                '820x800',  $null,   @()),
    @('hex-list',               'hex-list',          '1320x800', $null,   @()),
    @('hex-list-narrow',        'hex-list',           '820x800',  $null,   @()),
    @('hex-recording',          'hex-list-recording', '1320x800', $null,   @()),
    @('hex-recording-narrow',   'hex-list-recording', '820x800',  $null,   @()),
    @('hex-strip',              'hex-list',           '560x900',  $null,   @()),
    @('hex-recording-strip',    'hex-list-recording', '560x900',  $null,   @())
)
# The screens from the Granola tour: meeting tabs, transcript panel, "..." menu, Today strip and
# the Ctrl+K palette. They read the invented meetings in tools/ui-fixtures/screens. Each renders
# wide, at 820 (the transcript covers the page) and at 560.
$screensScenes = @(
    'screens-summary', 'screens-notes', 'screens-transcript', 'screens-transcript-jump',
    'screens-more-menu', 'screens-toast', 'screens-home-more', 'screens-today',
    'screens-today-tasks', 'screens-today-quiet', 'screens-finder', 'screens-finder-query',
    'screens-finder-none', 'screens-finder-off', 'screens-ask-focused'
)
foreach ($n in $screensScenes) {
    $shots += ,@($n, $n, '1320x800', $null, @())
    $shots += ,@("$n-820", $n, '820x800', $null, @())
    $shots += ,@("$n-560", $n, '560x900', $null, @())
}
# Real keys and presses: Ctrl+K typing, Tab handing the query to Ask, T for the transcript, the
# Go to box, and the "..." button.
$shots += ,@('screens-finder-live', 'screens-finder-keys', '1320x800', 'screens-finder-live.json', @('--at', 'mark:end'))
$shots += ,@('screens-finder-tab', 'screens-today-quiet', '1320x800', 'screens-finder-tab.json', @('--at', 'mark:end'))
$shots += ,@('screens-transcript-key', 'screens-summary', '1320x800', 'screens-transcript-key.json', @('--at', 'mark:end'))
$shots += ,@('screens-transcript-time', 'screens-summary', '1320x800', 'screens-transcript-time.json', @('--at', 'mark:end'))
$shots += ,@('screens-more-open', 'screens-summary', '1320x800', 'screens-more-open.json', @('--at', 'mark:end'))
# The sidebar, live meeting, Ask bar and Hex sections. Each scene renders at 1320x800 and at
# 820x800 (where the sidebar starts folded unless the scene opens it), named sb-<scene> and
# sb-<scene>-820. tools/ui-fixtures/hex.json stands in for the engine in the live and hex ones.
$sidebarScenes = @(
    'list', 'home-open', 'home-collapsed', 'home-project', 'home-peers', 'home-end', 'tasks-open',
    'live-recording', 'live-starting', 'live-making', 'live-naming', 'recording-away',
    'live-notes-typed', 'live-panel', 'live-panel-empty', 'live-making-notes', 'meeting-your-notes',
    'ask-idle', 'ask-focused', 'ask-typed', 'ask-suggestions', 'ask-model', 'ask-focused-meeting',
    'ask-model-missing', 'ask-no-agent', 'ask-no-agent-again',
    'hex', 'hex-vocabulary', 'hex-general', 'hex-dictation', 'hex-meetings', 'hex-shortcuts',
    'hex-advanced', 'hex-advanced-open', 'hex-search'
)
foreach ($n in $sidebarScenes) {
    $shots += ,@("sb-$n", $n, '1320x800', $null, @())
    $shots += ,@("sb-$n-820", $n, '820x800', $null, @())
}
# A phone-width window: the sidebar is a rail, and a drawer when opened.
foreach ($n in 'home-open', 'live-recording', 'live-notes-typed', 'live-panel', 'live-panel-empty', 'live-making-notes', 'meeting-your-notes', 'hex', 'hex-general', 'hex-dictation', 'hex-meetings', 'hex-vocabulary', 'ask-idle', 'ask-focused', 'ask-typed', 'ask-model-missing', 'ask-no-agent') { $shots += ,@("sb-$n-560", $n, '560x900', $null, @()) }
# The sidebar on each page it serves, wide and at phone width (where it opens as a drawer).
# These scenes read a longer invented project list and a memory fixture with proposals.
foreach ($n in 'meetings', 'meeting', 'tasks', 'memory', 'check', 'settings', 'update', 'project', 'all', 'rail') {
    $shots += ,@("sidebar-$n", "sidebar-$n", '1320x800', $null, @())
    $shots += ,@("sidebar-$n-560", "sidebar-$n", '560x900', $null, @())
}
# The Ask bar on a meeting and across all meetings, answering, at phone width.
$shots += ,@('ask-narrow',         'ask',              '560x900', $null, @())
$shots += ,@('ask-running-narrow', 'ask-running',      '560x900', $null, @())
$shots += ,@('home-ask-running-narrow', 'home-ask-running', '560x900', $null, @())
# Which shots get a generated synthetic report, and with which options.
$v2 = @('--v2', '--triage', '--gaps', '--actions')
$mem = @('--v2', '--memory', '--proposals')
$checkFixture = @{
    'screens-today' = $v2; 'screens-today-820' = $v2; 'screens-today-560' = $v2;
    'check' = $v2; 'check-panel' = $v2; 'check-applied' = $v2; 'check-snooze' = $v2; 'check-click' = $v2;
    'check-keys' = $v2; 'check-dismiss' = $v2; 'check-pick-date' = $v2; 'check-details' = $v2;
    'check-changed' = $v2; 'check-story' = $v2; 'check-history' = $v2; 'check-picker' = $v2;
    'check-cite' = $v2; 'check-back' = $v2; 'check-home-card' = $v2; 'check-narrow' = $v2;
    'check-panel-narrow' = $v2; 'check-story-narrow' = $v2; 'check-history-narrow' = $v2;
    'check-home-card-narrow' = $v2;
    'check-v1' = @('--actions'); 'check-v1-panel' = @('--actions');
    'memory' = $mem; 'memory-panel' = $mem; 'memory-review' = $mem; 'memory-retired' = $mem;
    'memory-forget' = $mem; 'memory-edit' = $mem; 'memory-confirm' = $mem; 'memory-accept-proposal' = $mem;
    'memory-narrow' = $mem; 'memory-panel-narrow' = $mem; 'memory-review-narrow' = $mem;
    'memory-migration' = @(); 'memory-skim' = @(); 'memory-migrated' = @(); 'memory-migration-narrow' = @()
}
foreach ($theme in $Themes) {
    foreach ($shot in $shots) {
        $name, $scene, $size, $script, $extra = $shot
        if ($Only.Count -gt 0 -and $Only -notcontains $name) { continue }
        $state = Join-Path ([IO.Path]::GetTempPath()) "meetings-ui-state-$name-$theme"
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        $state = [IO.Path]::GetFullPath($state)
        if (-not $state.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) { throw "Render state must stay inside the temporary folder" }
        Remove-Item -LiteralPath $state -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force -Path $state | Out-Null
        $projects = if ($scene -like 'sidebar-*') {
            @('Northwind Atlas', 'Northwind Field Guide', 'Harbor pricing study', 'Orchard onboarding', 'Lighthouse hiring',
              'Quarterly planning', 'Supplier review', 'Website refresh', 'Volunteer program', 'Field kit redesign')
        } else { @('Northwind Atlas', 'Northwind Field Guide') }
        @{ projects = $projects; aliases = @{} } |
            ConvertTo-Json -Depth 3 | Set-Content -Encoding utf8 "$state/projects.json"
        $enabled = $scene -notlike 'features-off-*' -and $scene -ne 'features-settings' -and $scene -ne 'features-tasks-only'
        @{ version = 1; features = @{ tasks = ($enabled -or $scene -eq 'features-tasks-only'); memory = $enabled; daily_check = $enabled; projects = $enabled; other_devices = $enabled } } |
            ConvertTo-Json -Depth 3 | Set-Content -Encoding utf8 "$state/config.json"
        if ($scene -eq 'ask') { Copy-Item "$root/tools/ui-fixtures/ask-answer.json" "$state/$meeting.json" }
        if ($scene -like 'sidebar-*') { $checkFixture[$name] = $mem }
        if ($checkFixture.ContainsKey($name)) { & python "$root/tools/prepare_check_fixture.py" $state @($checkFixture[$name]); if ($LASTEXITCODE -ne 0) { throw "No check fixture for $name" } }
        if ($scene -like 'tasks-cleared*') { Set-Content -Encoding utf8 "$state/tasks-settings.json" '{"cleared_before": "2026-10-31"}' }
        Copy-Item "$root/tools/ui-fixtures/hex.json" "$state/hex-fixture.json"
        if ($scene -eq 'home-ask') { Copy-Item "$root/tools/ui-fixtures/ask-all-answer.json" "$state/all.json" }
        $env:SIDENOTE_STATE_DIR = $state
        # The live and notes scenes read an invented meeting folder, never real meetings.
        $fixtureMeetings = if ($scene -like 'live-*' -or $scene -eq 'meeting-your-notes') { @('--meetings-dir', "$root/tools/ui-fixtures/live-notes/meetings") } elseif ($scene -like 'screens-*') { @('--meetings-dir', "$root/tools/ui-fixtures/screens/meetings") } else { @('--meetings-dir', "$root/tools/ui-fixtures/meetings") }
        $arguments = $fixtureMeetings + @('--data-dir', $state, '--snap', "$out/$name-$theme.png", '--scene', $scene, '--theme', $theme, '--size', $size, '--scale', '1') + $extra
        if ($script) { $arguments += @('--script', "$root/tools/ui-scripts/$script") }
        $log = "$out/render.log"
        $process = Start-Process -FilePath $Exe -ArgumentList $arguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput "$log.out" -RedirectStandardError $log -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Render $name-$theme failed with exit $($process.ExitCode), see $log" }
        Remove-Item -LiteralPath $state -Recurse -Force -ErrorAction SilentlyContinue
        Write-Output "$out/$name-$theme.png"
    }
}
Remove-Item Env:SIDENOTE_STATE_DIR -ErrorAction SilentlyContinue
