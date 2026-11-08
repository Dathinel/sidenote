# ADR 0008: Distribution and updates

Status: accepted, 2026-11-08. Signing depends on a public repository and SignPath approval.

## Context

- **Current manual installs** copy a build into `%LOCALAPPDATA%\Programs\hex-meetings` by hand.
- **Strangers need** an installer, safe updates and a signed binary.
- Managed-device policies may block unsigned executables. Signing does not guarantee acceptance; the device administrator must allow the publisher.
- **Constraints:** the app must work offline, and MCP clients and scheduled tasks need an executable path that does not change between versions.

## Decision

**Installer and updates: Velopack (MIT).**
- **Build:** a release workflow on `windows-latest` builds the exe, then runs `vpk pack`, which produces `Setup.exe`, full and delta packages and the release feed.
- **Startup:** `main` calls `velopack::VelopackApp::build().run()` first. It returns at once unless Velopack passed a hook argument, so the `--mcp` path stays fast.
- **Install:** per user, to `%LOCALAPPDATA%\hex-meetings\`. The stable path `%LOCALAPPDATA%\hex-meetings\current\hex-meetings.exe` is what MCP configs and scheduled tasks use.
- **Updates:**
  - The app checks GitHub Releases at startup and every 6 hours, on a stable or beta channel.
  - It downloads in the background and applies on the next restart, never while recording or while a job runs.
  - Offline, it simply skips the check.
- **Rollback:** publish the previous build again with a higher version number. Keep the last 3 full packages.
- **Moving from the manual install:** first run of the new build finds the old folder, shows the new path, and offers to update the listener task and MCP configs it knows. It never deletes the old folder.

**Signing.**
- **Default:** apply to SignPath Foundation once the repository is public under an OSI licence. It is free for qualifying open source projects, signs from hosted CI through its GitHub Action, and issues the certificate under SignPath Foundation with the project named.
- **Fallback:** Azure Artifact Signing, a monthly fee with identity checks. Verify eligibility for an individual or company in the maintainer's region before relying on it.
- **What gets signed:** `hex-meetings.exe`, `Setup.exe` and `Update.exe`, through `vpk`'s signing hook.
- **Until signing works,** builds stay private to the owner, with the Intune admin approving by hash.

**Supply chain.**
- `cargo-deny` checks licences and advisories, including the AGPL boundary from ADR 0004.
- `cargo-cyclonedx` produces an SBOM.
- GitHub artifact attestations record build provenance.
- Every build uses `--locked`.

**macOS later.** Prerequisites, in order:
1. Rune and Hex confirmed on macOS.
2. Platform adapters replace `windows-sys` uses: `platform.rs`, the `store_io` locks, `device::random_id` (move to `getrandom`), job objects (process groups) and named events.
3. Keychain through the `keyring` crate (already chosen in ADR 0003).
4. Velopack's macOS bundle, with codesign and notarization under an Apple Developer ID (yearly fee).

There is no macOS date until step 1 passes.

## Options rejected

- MSIX and the Microsoft Store: package sandbox and review friction for an app that reads `%APPDATA%\hex`.
- cargo-dist: good for CLI releases, but no built-in desktop auto-updater.
- cargo-packager: needs a separate updater.
- WiX MSI: no delta updates, and it is usually a machine-wide admin install.
- A self-written updater: security-critical code by hand.

## Consequences

- One CI workflow builds, packs, signs and publishes; updates are small deltas.
- A stable install path ends broken MCP configs after updates.
- Public signing waits on the privacy gate (review of fixtures and history, S04).

## Revisit if

- SignPath declines the project: switch to Azure Artifact Signing.
- The Velopack Rust SDK stops being maintained: compare cargo-packager with its updater plugin.

## First step

Roadmap lane D1: a release workflow that builds an unsigned Velopack package on a tag, plus a
CI job that installs version N and updates it to N+1 on a clean Windows runner.
