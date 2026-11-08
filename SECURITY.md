# Security

Report vulnerabilities privately through this repository's GitHub **Security >
Advisories > Report a vulnerability** form. The repository maintainer must enable
private vulnerability reporting before publication. If the form is unavailable,
wait for a private channel rather than opening a public issue with sensitive details.

Include a minimal reproduction using synthetic meeting data, the affected version,
and expected versus actual behavior. Never include credentials or real transcripts.
We will discuss fixes privately and publish an advisory when a fix is available.

Sidenote runs as the local user. Agent CLIs and MCP clients can access the supplied
meeting corpus; provider permissions do not create an OS filesystem boundary.
Only sync with fully trusted devices. Local state, backups and exported text are
sensitive. The current release does not promise encrypted sync or secure erasure.
