# Device-local engine settings and script inputs

Stage 2 stores choices only. It does not switch engines, run helpers, fetch a repository,
import custom script arguments, or read additional environment variables. All paths below
are under `scripts/meeting-pipeline/` at the stage-2 base, `436168d`.

`engine_settings.stages` is abbreviated `stages` below. A request deadline bounds an inner
helper/operation; its stage deadline bounds the enclosing task. These are separate budgets.
Notes helper/model and KB fields reuse their existing locations. An ASR model has one effective
home: an explicit `pipeline.after_stop_languages.routes[language]` wins, otherwise
`engine_settings.models[language]` retains a legacy Hex identifier. Editing the route must
clear the `models.te` migration status, including changes to its provider or deletion.

| Script setting and source lines | Canonical setting home |
| --- | --- |
| Translation provider/model: `hex-meeting-english.ps1:3,73-77`; OpenCode launcher `hex-pipeline-run.py:102-119` | `stages.translation_helper`, `stages.translation_model` |
| Translation timeout: `hex-meeting-english.ps1:7,73-77` | `stages.translation_deadline_seconds` (240) |
| Notes model: `hex-meeting-english.ps1:6,231`; `hex-meeting-notes.ps1:10,77` | Existing `notes.helper`, `notes.model` (dev.28 picker) |
| Notes timeout: `hex-meeting-notes.ps1:12,77`; outer stage `hex-meeting-english.ps1:230` | `stages.notes_deadline_seconds` (600) |
| Title provider/model/timeout: `hex-meeting-quick-title.ps1:9-10,70,84` | `stages.title_helper`, `title_model`, `title_deadline_seconds` (60) |
| Task provider/model/inner timeout: `hex-meeting-tasks.py:22,81-82` | `stages.tasks_helper`, `tasks_model`, `tasks_request_deadline_seconds` (300) |
| Task outer timeout: `hex-meeting-english.ps1:280-281` | `stages.tasks_deadline_seconds` (360) |
| Comparison provider/model/inner timeout: `hex-meeting-compare-granola.py:15-20` | `stages.comparison_helper`, `comparison_model`, `comparison_request_deadline_seconds` (600) |
| Comparison delay/outer timeout: `hex-meeting-compare-delayed.ps1:5,9` | `stages.comparison_delay_seconds` (300), `comparison_deadline_seconds` (1800) |
| Comparison KB/command timeout: `hex-meeting-compare-granola.py:10-12,179` | `stages.comparison_kb_deadline_seconds` (300) |
| Audio preparation: `hex-meeting-english.ps1:68-70,88-89` | `stages.prepare_deadline_seconds` (1800) |
| Routing observe/plan: `hex-meeting-english.ps1:92,96`; LID child `meeting-audio.py:133` | `stages.routing_deadline_seconds` (360), `lid_request_deadline_seconds` (300) |
| Transcription override: `hex-meeting-english.ps1:8,108-115` | Existing `engine_settings.stage_deadline_seconds`; exclusively transcription, None/0 means computed |
| ASR computed deadline rate/headroom: `hex-meeting-english.ps1:9-10,109` | `stages.asr_seconds_per_audio_second_per_pass` (0.20), `asr_deadline_headroom` (1.5) |
| Per-ASR HTTP request: `meeting-audio.py:479` | `stages.asr_request_deadline_seconds` (180) |
| ASR language models: `meeting-audio.py:325-326`; English reading `hex-meeting-english.ps1:135-168` | Existing language routes or legacy fallback, per language, including en/hi/te |
| Diagnostic route/all-model defaults: `hex-meeting-english.ps1:14-15,114`; `meeting-audio.py:561-562` | `stages.routing_mode` (shadow), `stages.all_models` (false) |
| Rendering/publishing: `hex-meeting-english.ps1:269-275` | `stages.render_deadline_seconds`, `publish_deadline_seconds` (120 each) |
| Explicit KB fetch / roster read: `hex-meeting-notes.ps1:61-62`; project-list read `hex-meeting-tasks.py:48-49` | `stages.kb_refresh_deadline_seconds` (60), `kb_read_deadline_seconds` (30) |
| KB repository / revision / roster path: `hex-meeting-notes.ps1:11,62` | Existing `knowledge_base.repo_path`, `revision`, `people_roster_source`; no automatic fetch |
| Project list / comparison playbook paths: `hex-meeting-tasks.py:48`; `hex-meeting-compare-granola.py:179` | `stages.project_list_source`, `comparison_playbook_source`; both use the existing KB revision |
| Allowed export roots: `hex-meeting-render.py:50-55` | `stages.export_roots`; empty delegates to the platform Documents/Hex Meetings default at later integration |
| Eight legacy values, aliases and transcription timeout | Existing ten-name table in `src/core/engine_settings/migration.rs`; no new environment reads |
| Notes / translation instructions: `hex-meeting-english.ps1:170-216,225` | Existing local `engine_settings.notes_prompt`, `translation_prompt`, with reset to shipped defaults |

The computed ASR formula retains its fixed startup allowance (300) and clamps (1800..3600)
from `hex-meeting-english.ps1:109`; these are algorithm constants, not additional overrides.
The worker-thread setting remains `engine_settings.worker_threads` (4).

Meeting directories, input/output files, `NotesOnly`, force/rerun targets, project overrides,
comparison time windows and supplied comparison CSVs are per-meeting/job inputs, not device
settings. `RoutingConfig` supplies an explicit pipeline snapshot for an experiment; the
persistent homes are the existing `pipeline` and `labs` sections. Executable locations, temporary folders,
process cleanup, audio segmentation constants and model catalog/checksum definitions remain
runtime/algorithm contracts, not duplicated user settings. Custom scripts and their files are
preserved; porting arbitrary script edits is not established by this schema.

Recovery uses `Config::engine_backup_names(root)` and explicit
`Config::restore_engine_backup(root, name)`. Neither operation deserializes the primary
config. Names are restricted to generated v1 snapshot names; restoration validates and
bounds the selected snapshot before atomically replacing the primary under its writer lock.
Snapshots are retained, including an orphan produced before a failed primary publication.
Ordinary config saves validate explicitly edited fields; unavailable preserved helper choices
remain stored and appear as availability status rather than a reason to reject another edit.
