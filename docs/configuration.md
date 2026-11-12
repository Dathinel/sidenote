# Private configuration

`config.json` is device-local and must never contain credentials. Existing feature
keys and unknown fields survive feature edits. Missing KB fields default to empty.
No company, person, roster, glossary or repository location is built into daily check.

```json
{
  "version": 1,
  "features": {},
  "knowledge_base": {
    "repo_path": "",
    "revision": "",
    "source_roots": [],
    "company_name": "",
    "people_roster_source": "",
    "glossary": ""
  }
}
```

Set `repo_path` to your KB checkout, `revision` to its tracked `origin/<branch>`,
and `source_roots` to explicit repository-relative files or folders to snapshot.
Set `company_name` to the organization to use as context. `people_roster_source`
is an optional repository-relative text source, included in the snapshot. `glossary`
is optional plain text. Daily check refuses to run until a repository, remote
revision and nonempty source roots are configured. Invalid paths and empty
snapshots fail before any model call. It can reuse a previously captured snapshot
after a fetch failure, with a visible stale warning.

Use source roots that contain only the material you intend to send to your agent.
Daily check fetches the configured repository and writes local snapshots and reports;
it does not write KB content. Restoring your prior config restores your prior choices.

People loads owner identity and company from config at startup. Roster loading starts
in the background after the first presented frame. The parsed Markdown roster is
cached by repository path, resolved commit ID and roster path; warm refreshes read
loose or packed local Git refs and launch no Git process. Worktree gitdir and
commondir pointers are supported. Git is used only on a cache miss.

People supports Markdown only; YAML, JSON and TXT are unsupported. This is explained
in Knowledge base settings without leaving a permanent People alert. The first
table with a `Person` column and a role column (header starting with `Role`) is the
roster. Generic `Name` tables and subsequent tables are ignored. `/` and commas
separate explicitly listed spellings; an optional `Aliases` column lists aliases.
Parenthetical, square-bracket and brace annotations are stripped, never names.

Full-name and explicit alias matches are automatic. A standalone first name only
suggests Our team and requires Confirm; confirmation saves a manual side. The saved
`notes.your_name` identifies you, and your short name explicitly listed on your own
roster row is an alias. Other aliases on that row do not compete with it.

Manual side choices win and can be cleared with Reset to automatic. Confirm, side
changes and reset merge only that person's side fields and preserve unrelated data.
Manual External hides the company label. Automatic classifications are display-time
projections; no backfill or config save is needed.
