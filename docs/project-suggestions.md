# Project suggestions

Model output belongs in each meeting's `tasks.json`: `meeting_project` plus each
task's `project`, `new` and `mine`. The model's `new` flag does not grant permission
to create a project. The app checks its registered project names instead.

Previously `hex-meeting-tasks.py` appended unknown task project names to the local
`projects.json` in `classify`, then called `save_known`. The Granola importer's
`import_one` also copied `record["project_names"]` into that registry using
`add_list_entry`. In the app, `project_names` also listed every pipeline name.

Now classification writes suggestions only to `tasks.json`. An unknown name leaves
tasks Unsorted and appears on the home and meeting rows as New project: name.
Create registers it and applies the per-task project overrides. The chip's menu
lets the user choose an existing project instead. Existing names retain the accept
tick. Dismiss writes Unsorted overrides tied to the task text, so reloading or
reclassifying the same task cannot bring that suggestion back. If a regenerated
summary changes the task text, old overrides no longer apply, as before.
All these meeting actions return immediately while Projects is off.
Existing legacy registered names remain; this change does not delete user data.

External pipeline helpers must preserve the same suggestion-only behavior.
