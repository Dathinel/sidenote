param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    # Another output folder (for example a before set), and an optional list of shot names.
    [string]$Out = "",
    [switch]$Dev3,
    [switch]$SyncProofs,
    [ValidateRange(1, 4)][int]$Throttle = 1,
    [string[]]$Only = @(),
    # Themes to render; dark first is enough for a quick look.
    [string[]]$Themes = @('dark', 'light')
    , [switch]$Trees
    , [string]$StateRoot = ""
    , [ValidateRange(0.5, 3.0)][double]$Scale = 1.0
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
    @('relay-preview', 'relay-preview', '1280x800', $null, @()),
    @('relay-timeout', 'relay-timeout', '1280x800', $null, @()),
    @('relay-unconfirmed', 'relay-unconfirmed', '1280x800', $null, @()),
    @('relay-operational', 'relay-operational', '1280x800', $null, @()),
    @('relay-unreachable', 'relay-unreachable', '1280x800', $null, @()),
    @('relay-wrong-address', 'relay-wrong-address', '1280x800', $null, @()),
    @('relay-update-required', 'relay-update-required', '1280x800', $null, @()),
    @('relay-pair-code', 'relay-pair-code', '1280x800', $null, @()),
    @('relay-pair-confirm', 'relay-pair-confirm', '1280x800', $null, @()),
    @('relay-paired', 'relay-paired', '1280x800', $null, @()),
    @('relay-removal-unavailable', 'relay-paired', '1280x800', $null, @()),
    @('relay-conflict', 'relay-conflict', '1280x800', $null, @()),
    @('relay-shared-conflict', 'relay-shared-conflict', '1280x800', $null, @()),
    @('relay-shared-resolved', 'relay-shared-conflict', '1280x800', 'relay-shared-resolve.json', @('--at','mark:end')),
    @('relay-entry', 'relay-preview', '1280x800', 'relay-entry.json', @('--at','mark:end')),
    @('relay-conflict-resolved', 'relay-conflict', '1280x800', 'relay-resolve.json', @('--at','mark:end')),
    @('profile-menu-narrow', 'profile-menu', '560x900', $null, @()),
    @('tasks-v2-owner-narrow', 'tasks-v2-owner', '560x1000', $null, @()),
    @('tasks-v2-legacy-panel', 'tasks-v2-legacy-panel', '1320x1000', $null, @()),
    @('tasks-v2-long-panel', 'tasks-v2-long-panel', '900x900', $null, @()),
    @('tasks-v2-long-panel-560', 'tasks-v2-long-panel', '560x900', $null, @()),
    @('tasks-v2-titles-edited', 'tasks-v2-legacy-panel', '1320x1000', 'tasks-titles-edit.json', @('--at', 'mark:end')),
    @('tasks-v2-full-titles', 'tasks-v2-owner', '1320x1000', 'tasks-full-titles.json', @('--at', 'mark:end')),

    @('profile-menu', 'profile-menu', '1320x800', $null, @()),
    @('profile-editor', 'profile-menu', '1320x1000', 'profile-editor.json', @('--at', 'mark:end')),
    @('prompt-editor-link', 'notes-meeting-bad-output', '1320x1000', $null, @()),
    @('mcp-connect', 'mcp-connect', '1320x1500', $null, @()),
    @('mcp-connect-560', 'mcp-connect', '560x1800', $null, @()),
    @('listener-error', 'listener-error', '1320x900', $null, @()),
    @('listener-error-narrow', 'listener-error', '560x1000', $null, @()),
    @('live-close-failure', 'live-close-failure', '1320x900', $null, @()),
    @('live-close-failure-narrow', 'live-close-failure', '560x1000', $null, @()),
    @('tasks-v2-edited', 'tasks-v2-edited', '1320x1300', 'tasks-v2-edit.json', @('--at', 'mark:end')),
    @('tasks-v2-edited-560', 'tasks-v2-edited', '560x1400', 'tasks-v2-edit.json', @('--at', 'mark:end')),
    @('tasks-v2-auto', 'tasks-v2-auto', '1320x1300', 'tasks-v2-auto.json', @('--at', 'mark:end')),
    @('tasks-v2-auto-560', 'tasks-v2-auto', '560x1400', 'tasks-v2-auto.json', @('--at', 'mark:end')),
    @('tasks-v2-team-saved', 'tasks-v2-team-saved', '1320x1300', 'tasks-v2-team.json', @('--at', 'mark:end')),
    @('tasks-v2-team-saved-560', 'tasks-v2-team-saved', '560x1400', 'tasks-v2-team.json', @('--at', 'mark:end')),
    @('tasks-v2-project', 'tasks-v2-project', '1320x1300', $null, @()),
    @('tasks-v2-project-560', 'tasks-v2-project', '560x1400', $null, @()),
    @('tasks-v2-team', 'tasks-v2-team', '1320x1300', $null, @()),
    @('tasks-v2-team-560', 'tasks-v2-team', '560x1400', $null, @()),
    @('tasks-v2-status', 'tasks-v2-status', '1320x1300', $null, @()),
    @('tasks-v2-status-560', 'tasks-v2-status', '560x1400', $null, @()),
    @('tasks-v2-owner', 'tasks-v2-owner', '1320x1300', $null, @()),
    @('tasks-v2-owner-560', 'tasks-v2-owner', '560x1400', $null, @()),
    @('tasks-v2-due', 'tasks-v2-due', '1320x1300', $null, @()),
    @('tasks-v2-due-560', 'tasks-v2-due', '560x1400', $null, @()),
    @('tasks-v2-panel', 'tasks-v2-panel', '1320x1300', $null, @()),
    @('tasks-v2-panel-560', 'tasks-v2-panel', '560x1400', $null, @()),
    @('tasks-v2-waiting', 'tasks-v2-waiting', '1320x1300', $null, @()),
    @('tasks-v2-waiting-560', 'tasks-v2-waiting', '560x1400', $null, @()),
    @('tasks-v2-review', 'tasks-v2-review', '1320x1300', $null, @()),
    @('tasks-v2-review-560', 'tasks-v2-review', '560x1400', $null, @()),
    @('tasks-v2-legacy', 'tasks-v2-legacy', '1320x1300', $null, @()),
    @('tasks-v2-legacy-560', 'tasks-v2-legacy', '560x1400', $null, @()),
    @('tasks-v2-empty', 'tasks-v2-empty', '1320x1300', $null, @()),
    # The board layout: four columns wide, one column under tabs at phone width, and no tasks.
    @('tasks-board', 'tasks-v2-board', '1320x1000', $null, @()),
    @('tasks-board-390', 'tasks-v2-board', '390x1500', $null, @()),
    @('tasks-board-empty', 'tasks-v2-board-empty', '1320x800', $null, @()),
    @('tasks-board-drag', 'tasks-v2-board-drag', '1320x1000', $null, @()),
    @('tasks-board-filter', 'tasks-v2-board-filter', '1320x1000', $null, @()),
    @('tasks-v2-empty-560', 'tasks-v2-empty', '560x1400', $null, @()),
    @('tasks-v2-teams', 'tasks-v2-teams', '1320x1300', $null, @()),
    @('tasks-v2-teams-560', 'tasks-v2-teams', '560x1400', $null, @()),
    @('sync-memory-merged', 'sync-memory-merged', '1320x900', $null, @()),
    @('sync-task-merged', 'sync-task-merged', '1320x900', $null, @()),
    @('sync-task-kept', 'sync-task-kept', '1320x900', $null, @()),
    @('sync-arrival-draft', 'sync-arrival-draft', '1320x900', $null, @()),
    @('sync-footer-now', 'sync-footer-now', '1320x900', $null, @()),
    @('sync-footer-age', 'sync-footer-age', '1320x900', $null, @()),
    @('sync-footer-off', 'sync-footer-off', '1320x900', $null, @()),

    @('areas-refresh-error', 'areas-refresh-error', '1320x900', $null, @()),
    @('areas-refresh-error-560', 'areas-refresh-error', '560x1000', $null, @()),
    @('areas-large-reconcile', 'areas-large-reconcile', '1320x900', $null, @()),
    @('areas-large-reconcile-560', 'areas-large-reconcile', '560x1000', $null, @()),
    @('areas-large-empty', 'areas-large-empty', '1320x900', $null, @()),
    @('areas-large-empty-560', 'areas-large-empty', '560x1000', $null, @()),
    @('areas-large-page', 'areas-large-page', '1320x1100', $null, @()),
    @('areas-large-page-560', 'areas-large-page', '560x1400', $null, @()),
    @('areas-large-convert', 'areas-large-convert', '1320x1100', $null, @()),
    @('areas-large-convert-560', 'areas-large-convert', '560x1400', $null, @()),
    @('merge-worker-proof', 'merge-worker-proof', '1320x1000', $null, @()),
    @('merge-preparing', 'merge-preparing', '1320x1000', $null, @()),
    @('merge-preparing-560', 'merge-preparing', '560x1100', $null, @()),
    @('merge-merging', 'merge-merging', '1320x1000', $null, @()),
    @('merge-merging-560', 'merge-merging', '560x1100', $null, @()),
    @('merge-undoing', 'merge-undoing', '1320x1000', $null, @()),
    @('merge-undoing-560', 'merge-undoing', '560x1100', $null, @()),
    @('merge-preview-expanded', 'merge-preview-expanded', '1320x1300', $null, @()),
    @('merge-preview-expanded-560', 'merge-preview-expanded', '560x1600', $null, @()),
    @('merge-page', 'merge-page', '1320x1000', $null, @()),
    @('merge-page-560', 'merge-page', '560x1100', $null, @()),
    @('merge-picker', 'merge-picker', '1320x1000', $null, @()),
    @('merge-picker-560', 'merge-picker', '560x1100', $null, @()),
    @('merge-preview', 'merge-preview', '1320x1300', $null, @()),
    @('merge-preview-560', 'merge-preview', '560x1600', $null, @()),
    @('merge-done', 'merge-done', '1320x1100', $null, @()),
    @('merge-done-560', 'merge-done', '560x1300', $null, @()),
    @('project-choices', 'project-choices', '1320x1100', $null, @()),
    @('project-choices-560', 'project-choices', '560x1300', $null, @()),
    @('project-before-560', 'home-project', '560x1100', $null, @()),
    @('suggestion-before-560', 'audio-project-new', '560x1100', $null, @()),
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
    # The Diary: invented days from prepare_check_fixture.py --diary (meetings 3 to 6 Nov, memory
    # changes, three checks, two with a day summary), today with two ticks, the newest check's findings, the finding
    # panel, the day picker, an older day's full report, the sidebar and an empty diary. The
    # tall shots show the whole feed in one picture.
    @('diary',              'diary',         '1320x800',  $null, @()),
    @('diary-tall',         'diary',         '1320x2400', $null, @()),
    @('diary-panel',        'diary-panel',   '1320x800',  $null, @()),
    @('diary-picker',       'diary-picker',  '1320x800',  $null, @()),
    # Every section open: tasks created, memories created, done, needs your attention and lows.
    @('diary-open-tall',    'diary-open',    '1320x2400', $null, @()),
    @('diary-open-narrow-tall', 'diary-open', '560x3200', $null, @()),
    @('diary-older',        'diary-older',   '1320x800',  $null, @()),
    @('diary-report',       'diary-report',  '1320x800',  $null, @()),
    @('diary-sidebar',      'diary-sidebar', '1320x800',  $null, @()),
    @('diary-sidebar-560',  'diary-sidebar', '560x900',   $null, @()),
    @('diary-narrow',       'diary',         '560x900',   $null, @()),
    @('diary-narrow-tall',  'diary',         '560x3200',  $null, @()),
    @('diary-panel-narrow', 'diary-panel',   '560x900',   $null, @()),
    @('diary-empty',        'diary',         '1320x800',  $null, @()),
    @('diary-empty-narrow', 'diary',         '560x900',   $null, @()),
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
foreach ($areaScene in @('areas-page','areas-all','areas-project','areas-list','areas-migration','areas-migration-empty','areas-convert','areas-create','areas-rename','areas-move','areas-assign','areas-worker-proof')) {
    $shots += ,@($areaScene, $areaScene, '1320x1100', $null, @())
    $shots += ,@("$areaScene-560", $areaScene, '560x1400', $null, @())
}
foreach ($areaScene in @('areas-large-migration','areas-large-review','areas-large-expanded')) {
    $shots += ,@($areaScene, $areaScene, '1320x900', $null, @())
    $shots += ,@("$areaScene-560", $areaScene, '560x1000', $null, @())
}
# Ask model and thinking controls at the requested review sizes.
foreach ($width in @(1280,640)) {
    $shots += ,@("models-and-agents-$width", 'settings-helpers', "${width}x800", $null, @())
    $shots += ,@("models-and-agents-menu-$width", 'settings-model-menu', "${width}x800", $null, @())
    $shots += ,@("models-and-agents-other-$width", 'settings-model-other', "${width}x800", $null, @())
    foreach ($scene in @('ask-thinking-picker','ask-in-meeting','ask-streaming','ask-opencode')) {
        $shots += ,@("$scene-$width", $scene, "${width}x800", $null, @())
    }
}
# The screens from the Granola tour: meeting tabs, transcript panel, "..." menu, Today strip and
# the Ctrl+K palette. They read the invented meetings in tools/ui-fixtures/screens. Each renders
# wide, at 820 (the transcript covers the page) and at 560.
$screensScenes = @(
    'screens-summary', 'screens-notes', 'screens-transcript', 'screens-transcript-jump',
    'screens-more-menu', 'screens-toast', 'screens-home-more', 'screens-today',
    'screens-today-tasks', 'screens-today-quiet', 'screens-today', 'screens-today-tasks', 'screens-finder', 'screens-finder-query',
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
    'live-finishing', 'live-interrupted', 'live-making-slow',
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
foreach ($n in 'live-finishing', 'live-interrupted', 'live-making-slow') { $shots += ,@("sb-$n-560", $n, '560x900', $null, @()) }
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
# The dev3 matrix is one reproducible run, including the reviewer's unopened scenes.
if ($Dev3) {
    $shots = @($shots | Where-Object { $_[0] -like 'tasks-v2-*' -or $_[0] -like 'listener-error*' -or $_[0] -like 'live-close-failure*' })
    $source = Get-Content "$root/src/ui/mod.rs" -Raw
    $notes = [regex]::Matches($source, '"(notes-(?:settings|meeting|live|today|list)-[^" ]+)"') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique
    foreach ($scene in $notes) {
        foreach ($width in @(1320, 560)) {
            $height = if ($scene -like 'notes-settings-fixed-*') { 3400 } elseif ($scene -like 'notes-settings-*') { 1600 } elseif ($scene -like 'notes-list-*') { 1800 } else { 1100 }
            $shots += ,@("$scene-$width", $scene, "${width}x${height}", $null, @())
        }
    }
    foreach ($interaction in @(
        @('notes-click-ask', 'notes-today-first-open', 'notes-ask.json'),
        @('notes-click-dismiss', 'notes-meeting-fallback', 'notes-dismiss.json'),
        @('notes-click-reset', 'notes-settings-edited', 'notes-reset-confirm.json'),
        @('notes-click-fixed', 'notes-settings-tasks', 'notes-show-fixed.json'),
        @('notes-click-privacy', 'notes-today-found', 'notes-privacy.json'),
        @('notes-click-teams', 'notes-settings-tasks', 'notes-teams.json')
    )) {
        foreach ($width in @(1320, 560)) {
            $height = if ($interaction[0] -eq 'notes-click-fixed') { 3400 } else { 1600 }
            $shots += ,@("$($interaction[0])-$width", $interaction[1], "${width}x${height}", $interaction[2], @('--at', 'mark:end'))
        }
    }
    foreach ($scene in @('live-recording', 'live-starting', 'live-notes-typed', 'live-panel', 'live-panel-empty', 'live-panel-closed', 'live-making-notes', 'live-making', 'live-naming', 'recording-away', 'meeting-your-notes', 'screens-summary', 'screens-transcript', 'screens-today-quiet', 'screens-today', 'screens-today-tasks', 'screens-finder', 'screens-finder-query', 'screens-finder-none', 'screens-finder-off', 'screens-more-menu', 'screens-home-more', 'ask-no-agent', 'ask-no-agent-again', 'features-settings', 'tasks-v2-clear-confirm', 'tasks-v2-rebuild-confirm', 'tasks-v2-waiting-edit')) {
        foreach ($width in @(1320, 560)) {
            $closeDrawer = $scene -eq 'features-settings' -and $width -eq 560
            $script = if ($closeDrawer) { 'dev3-close-drawer.json' } else { $null }
            $extra = if ($closeDrawer) { @('--at', 'mark:end') } else { @() }
            $shots += ,@("$scene-$width", $scene, "${width}x1400", $script, $extra)
        }
    }
}
if ($Dev3) {
    foreach ($width in @(1320, 560)) {
        $shots += ,@("tasks-v2-rebuild-blocked-$width", 'tasks-v2-rebuild-confirm', "${width}x1400", 'dev3-rebuild-blocked.json', @('--at', 'mark:end'))
    }
}
foreach ($scene in @('settings-kb-empty', 'settings-kb-valid', 'settings-kb-invalid', 'settings-kb-merge-error')) {
    $shots += ,@($scene, $scene, '1320x1200', $null, @())
    $shots += ,@("$scene-390", $scene, '390x1400', $null, @())
}
$settingsScenes = @(
    'settings-general',
    'settings-recording',
    'settings-summaries',
    'settings-tasks',
    'settings-prompts',
    'settings-people',
    'settings-hex',
    'settings-helpers',
    'settings-speech',
    'settings-helper-advanced',
    'settings-appearance',
    'settings-sync',
    'settings-updates',
    'settings-search',
    'settings-sidebar-open',
    'settings-sidebar-folded',
    'settings-recording-expanded',
    'settings-summaries-expanded',
    'settings-tasks-expanded',
    'settings-hex-expanded',
    'settings-helper-advanced-expanded',
    'settings-sync-expanded',
    'settings-updates-expanded',
    'settings-helpers-expanded'
)
foreach ($scene in $settingsScenes) {
    foreach ($width in @(1280, 640)) {
        $script = if ($scene -eq 'settings-search') { 'settings-search.json' } else { $null }
        $extra = if ($script) { @('--at', 'mark:end') } else { @() }
        $shots += ,@("${scene}-${width}", $scene, "${width}x800", $script, $extra)
    }
}
$shots += ,@('settings-review-sidebar-slash','settings-sidebar-open','1280x800','settings-review-slash.json',@('--at','mark:end'))
$shots += ,@('settings-review-settings-slash','settings-recording','1280x800','settings-review-slash.json',@('--at','mark:end'))
foreach ($scene in @('annotations-list', 'annotation-capture', 'annotations-empty', 'annotation-transcript','annotation-review','annotation-drafting','annotation-note')) {
    foreach ($width in @(1280, 640)) {
        $script = if ($width -eq 640) { 'dev3-close-drawer.json' } else { $null }
        $extra = if ($script) { @('--at', 'mark:end') } else { @() }
        $shots += ,@("${scene}-${width}", $scene, "${width}x800", $script, $extra)
    }
}
foreach ($scene in @('notebooks','notebook','notebook-note','share-confirmation')) {
    foreach ($width in @(1280,640)) {
        $script = if ($width -eq 640) { 'dev3-close-drawer.json' } else { $null }
        $extra = if ($script) { @('--at','mark:end') } else { @() }
        $shots += ,@("${scene}-${width}", $scene, "${width}x800", $script, $extra)
    }
}
if ($SyncProofs) {
    $syncProofScenes = @($shots | Where-Object { $_[1] -like 'relay-*' -or $_[1] -like 'sync-*' -or $_[1] -in @('settings-sync', 'settings-sync-expanded') })
    $shots = @()
    $seen = @{}
    foreach ($shot in $syncProofScenes) {
        $name = $shot[0] -replace '-(1280|640)$', ''
        if ($seen.ContainsKey($name)) { continue }
        $seen[$name] = $true
        foreach ($width in @(1280, 390)) {
            $shots += ,@("${name}-${width}", $shot[1], "${width}x800", $shot[3], $shot[4])
        }
    }
}
$priorAppData = $env:APPDATA
$priorLocalAppData = $env:LOCALAPPDATA
$priorState = $env:SIDENOTE_STATE_DIR
$rows = [System.Collections.Generic.List[object]]::new()
$pending = [System.Collections.Generic.Queue[object]]::new()
function Complete-Render($job) {
    $job.Process.WaitForExit()
    if ($job.Process.ExitCode -ne 0) { throw "Render $($job.Name) failed with exit $($job.Process.ExitCode), see $($job.Log)" }
    $rows.Add($job.Row)
    $rows | ConvertTo-Json -Depth 3 | Set-Content -Encoding UTF8 "$out/render-index.json"
    if ($job.Row.SourceScene -eq 'share-confirmation') {
        $html = Get-ChildItem -LiteralPath $job.State -File -Filter '*.html' | Select-Object -First 1
        if (-not $html) { throw 'The share confirmation did not produce HTML.' }
        & python "$root/tools/verify_notebook_export.py" $job.State $html.FullName "$out/export-proof-$($job.Name).json"
        if ($LASTEXITCODE -ne 0) { throw 'Export verification failed.' }
        Copy-Item -LiteralPath $html.FullName -Destination "$out/notebook.html" -Force
    }
    Remove-Item -LiteralPath $job.State -Recurse -Force -ErrorAction SilentlyContinue
    Write-Output $job.Row.Path
}
# Which shots get a generated synthetic report, and with which options.
$v2 = @('--v2', '--triage', '--gaps', '--actions')
$mem = @('--v2', '--memory', '--proposals')
$diary = @('--v2', '--triage', '--actions', '--memory', '--proposals', '--diary')
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
try {
foreach ($theme in $Themes) {
    foreach ($shot in $shots) {
        $name, $scene, $size, $script, $extra = $shot
        if ($Only.Count -gt 0 -and $Only -notcontains $name) { continue }
        $stateBase = if ($StateRoot) { $StateRoot } else { [IO.Path]::GetTempPath() }
        New-Item -ItemType Directory -Force -Path $stateBase | Out-Null
        # Device journals add two long identity/hash directories. Keep custom roots short.
        $stateName = if ($StateRoot) { [IO.Path]::GetRandomFileName() } else { "meetings-ui-state-$name-$theme" }
        $state = Join-Path $stateBase $stateName
        $tempRoot = [IO.Path]::GetFullPath($stateBase).TrimEnd('\') + '\'
        $state = [IO.Path]::GetFullPath($state)
        if (-not $state.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) { throw "Render state must stay inside the temporary folder" }
        Remove-Item -LiteralPath $state -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force -Path $state | Out-Null
        $projects = if ($scene -like 'areas-large-*') { @(1..14 | ForEach-Object { 'Example project {0:00}' -f $_ }) } elseif ($scene -like 'sidebar-*') {
            @('Northwind Atlas', 'Northwind Field Guide', 'Harbor pricing study', 'Orchard onboarding', 'Lighthouse hiring',
              'Quarterly planning', 'Supplier review', 'Website refresh', 'Volunteer program', 'Field kit redesign')
        } else { @('Northwind Atlas', 'Northwind Field Guide') }
        @{ projects = $projects; aliases = @{} } |
            ConvertTo-Json -Depth 3 | Set-Content -Encoding utf8 "$state/projects.json"
        if ($scene -like 'areas-large-*') {
            $utf8 = New-Object System.Text.UTF8Encoding($false)
            for ($projectIndex = 0; $projectIndex -lt 14; $projectIndex++) {
                $id = '{0}-1' -f (1800000010000L + $projectIndex)
                $folder = Join-Path "$state/large-meetings" $id
                New-Item -ItemType Directory -Force $folder | Out-Null
                $count = if ($projectIndex -lt 2) { 58 } else { 57 }
                $todos = @(for ($taskIndex = 0; $taskIndex -lt $count; $taskIndex++) { @{ task = ('Synthetic task {0:00}.{1:00}: review the paper sample' -f ($projectIndex + 1), ($taskIndex + 1)) } })
                $tags = @(for ($taskIndex = 0; $taskIndex -lt $count; $taskIndex++) { @{ i = $taskIndex; project = $projects[$projectIndex]; mine = $true; new = $false } })
                [IO.File]::WriteAllText("$folder/english-summary.json", (@{ title = ('Example planning {0:00}' -f ($projectIndex + 1)); kind = 'Synthetic meeting'; people = @('Alex','Morgan'); todo = $todos } | ConvertTo-Json -Depth 8), $utf8)
                [IO.File]::WriteAllText("$folder/tasks.json", (@{ meeting_project = $projects[$projectIndex]; tasks = $tags } | ConvertTo-Json -Depth 8), $utf8)
                [IO.File]::WriteAllText("$folder/manifest.json", (@{ created_at_ms = 1800000010000L + $projectIndex } | ConvertTo-Json), $utf8)
            }
        }
        if ($scene -like 'merge-*') {
            New-Item -ItemType Directory -Force -Path "$state/memory" | Out-Null
            @{ schema_version = 1; records = @(@{
                id = 'synthetic-merge-memory'; type = 'project'; text = 'Use paper cards for the Northwind workshop.';
                project = 'Northwind Field Guide'; sources = @(); first_seen = $null; last_confirmed = $null;
                status = 'active'; basis = 'owner_confirmed'; supersedes = $null;
                updated_at = '2026-11-09T00:00:00Z'; source_fingerprint = 'synthetic'
            }); proposals = @(); events = @(); migration = $null } |
                ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 "$state/memory/records.json"
        }
        if ($scene -like 'tasks-v2-*') {
            & python "$root/tools/prepare_tasks_v2_fixture.py" $state
            if ($LASTEXITCODE -ne 0) { throw "Could not prepare tasks v2 fixture" }
        }
        $enabled = $scene -notlike 'features-off-*' -and $scene -ne 'features-settings' -and $scene -ne 'features-tasks-only'
        @{ version = 1; features = @{ tasks = ($enabled -or $scene -in @('features-tasks-only', 'features-settings')); memory = $enabled; daily_check = $enabled; projects = $enabled; other_devices = $enabled } } |
            ConvertTo-Json -Depth 3 | Set-Content -Encoding utf8 "$state/config.json"
        if ($scene -eq 'ask') { Copy-Item "$root/tools/ui-fixtures/ask-answer.json" "$state/$meeting.json" }
        if ($scene -like 'sidebar-*') { $checkFixture[$name] = $mem }
        if ($name -like 'diary*' -and $name -notlike 'diary-empty*') { $checkFixture[$name] = $diary }
        if ($checkFixture.ContainsKey($name)) { & python "$root/tools/prepare_check_fixture.py" $state @($checkFixture[$name]); if ($LASTEXITCODE -ne 0) { throw "No check fixture for $name" } }
        if ($scene -like 'tasks-cleared*') { Set-Content -Encoding utf8 "$state/tasks-settings.json" '{"cleared_before": "2026-10-31"}' }
        Copy-Item "$root/tools/ui-fixtures/hex.json" "$state/hex-fixture.json"
        if ($scene -eq 'home-ask') { Copy-Item "$root/tools/ui-fixtures/ask-all-answer.json" "$state/all.json" }
        New-Item -ItemType Directory -Force -Path "$state/appdata", "$state/localappdata", "$state/peers" | Out-Null
        $env:APPDATA = "$state/appdata"
        $env:LOCALAPPDATA = "$state/localappdata"
        $env:SIDENOTE_STATE_DIR = $state
        # The live and notes scenes read an invented meeting folder, never real meetings.
        $fixtureMeetings = if ($scene -in @('notes-today-first-open', 'notes-today-hex-missing')) { New-Item -ItemType Directory -Force -Path "$state/no-meetings" | Out-Null; @('--meetings-dir', "$state/no-meetings") } elseif ($scene -like 'notes-*') { @('--meetings-dir', "$root/tools/ui-fixtures/screens/meetings") } elseif ($scene -like 'tasks-v2-*') { @('--meetings-dir', "$state/tasks-v2-meetings") } elseif ($scene -like 'areas-large-*') { @('--meetings-dir', "$state/large-meetings") } elseif ($name -like 'diary-empty*') { New-Item -ItemType Directory -Force -Path "$state/no-meetings" | Out-Null; @('--meetings-dir', "$state/no-meetings") } elseif ($name -like 'diary*') { @('--meetings-dir', "$state/diary-meetings") } elseif ($scene -like 'live-*' -or $scene -eq 'meeting-your-notes') { @('--meetings-dir', "$root/tools/ui-fixtures/live-notes/meetings") } elseif ($scene -like 'screens-*') { @('--meetings-dir', "$root/tools/ui-fixtures/screens/meetings") } else { @('--meetings-dir', "$root/tools/ui-fixtures/meetings") }
        $arguments = $fixtureMeetings + @('--data-dir', $state, '--peers-dir', "$state/peers", '--snap', "$out/$name-$theme.png", '--scene', $scene, '--theme', $theme, '--size', $size, '--scale', $Scale.ToString([Globalization.CultureInfo]::InvariantCulture)) + $extra
        if ($Trees) { $arguments += @('--tree', "$out/$name-$theme.json") }
        if ($script) { $arguments += @('--script', "$root/tools/ui-scripts/$script") }
        $log = "$out/$name-$theme.log"
        $process = Start-Process -FilePath $Exe -ArgumentList $arguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput "$log.out" -RedirectStandardError $log -PassThru
        $null = $process.Handle
        $pending.Enqueue([PSCustomObject]@{ Process = $process; Name = "$name-$theme"; Log = $log; State = $state; Row = [PSCustomObject]@{ Scene = $name; SourceScene = $scene; Size = $size; Theme = $theme; Path = "$out/$name-$theme.png" } })
        if ($pending.Count -ge $Throttle) { Complete-Render ($pending.Dequeue()) }
    }
}
    while ($pending.Count -gt 0) { Complete-Render ($pending.Dequeue()) }
} finally {
    $env:APPDATA = $priorAppData
    $env:LOCALAPPDATA = $priorLocalAppData
    $env:SIDENOTE_STATE_DIR = $priorState
}
