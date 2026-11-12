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

People also reads the roster directly from the configured Git revision, so the file
need not be present in the KB working tree. Use a Markdown table with a `Person`
or `Name` column; `/`, commas and parentheses in that column list explicit name
spellings or aliases. Matching ignores case and repeated whitespace, and never
uses role text, email domains or partial names. Matched people appear on Our team
with `company_name` as their company. Unrecognized roster formats show a notice.

The saved `notes.your_name` identifies you. A short first-name spelling identifies
you only if the roster lists it explicitly and contains no competing name starting
with that first name. Microphone transcript labels `You` and `Me` also identify the
local speaker; microphone audio alone does not establish a person's name.

Automatic sides and companies are display-time projections, including for people
already visible in People. No backfill or config save is needed. Entries in
`people_sides` are manual overrides and always win; automatic evidence never enters
that map. Saved company links, including an explicit no-company choice, also win.
