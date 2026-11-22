# Contributing to Sidenote

Thank you for helping. This guide covers the first build, the checks to run, and how a
pull request is checked. If a step here fails on your machine, please open an issue.

## Where to start

- Issues labelled [good first issue](https://github.com/harshanandak/sidenote/labels/good%20first%20issue)
  are small and have clear acceptance criteria.
- Welcome help: bug reports with clear steps, fixes for those bugs, Linux support,
  documentation, accessibility, and tests that use synthetic data.
- For a larger change, open an issue first so we can agree on the approach.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) shows where each part of the code lives.

## The data rule

Use synthetic meeting data only. Never put real transcripts, real names, company
details, API keys or tokens in code, tests, fixtures, issues, pull requests,
screenshots or logs. If a bug needs a meeting to reproduce, write a short made-up one.

## Prerequisites

Not yet verified on a clean machine: the prerequisites, first build and first run below
come from the CI workflow and the repo's build files, not from a fresh install.

Both platforms need Git and Rust installed with [rustup](https://rustup.rs).
Python 3 is needed for the tool checks. Bun 1.4.4 is needed only for work in `relay/`.

**Windows.** The repo pins the `stable-x86_64-pc-windows-gnu` toolchain
(`rust-toolchain.toml`), so you also need a full MinGW GNU toolchain. CI uses MSYS2:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup component add rustfmt clippy --toolchain stable-x86_64-pc-windows-gnu
```

Install [MSYS2](https://www.msys2.org), run `pacman -S mingw-w64-x86_64-toolchain` in its
shell, then add `<msys2 folder>\mingw64\bin` to PATH (or set `RUST_GNU_BIN` to it).
`as.exe` and `dlltool.exe` must be found.

**Ubuntu.** Install the native build packages and the stable toolchain:

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libx11-xcb-dev
rustup toolchain install stable --component rustfmt --component clippy
```

Opening the window also needs a graphical session and Vulkan drivers (Mesa's software driver works).

## First build

**Windows** (PowerShell, from the repo root): run `./build.ps1`. It runs
`cargo +stable-x86_64-pc-windows-gnu build --workspace --locked --bins --target x86_64-pc-windows-gnu`.
The app is `target/x86_64-pc-windows-gnu/debug/sidenote.exe`.

**Ubuntu.** The checked-in Cargo config targets Windows, so override the toolchain
and target in each shell. The app is `target/x86_64-unknown-linux-gnu/debug/sidenote`.

```sh
export RUSTUP_TOOLCHAIN=stable
export CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu
cargo build --no-default-features --features desktop --target x86_64-unknown-linux-gnu --bin sidenote
```

## Run it with empty test folders

Point Sidenote at empty folders so it never reads or changes your own meetings.
Hex is not needed to open the window. If the app does not start, create the folders first.

```powershell
# Windows
$env:SIDENOTE_DATA_DIR = "$env:TEMP\sidenote-dev\state"
$env:SIDENOTE_HEX_DIR = "$env:TEMP\sidenote-dev\meetings"
$env:SIDENOTE_PEERS_DIR = "$env:TEMP\sidenote-dev\peers"
./target/x86_64-pc-windows-gnu/debug/sidenote.exe
```

```sh
# Ubuntu
root=$(mktemp -d)
SIDENOTE_DATA_DIR="$root/state" SIDENOTE_HEX_DIR="$root/meetings" SIDENOTE_PEERS_DIR="$root/peers" ./target/x86_64-unknown-linux-gnu/debug/sidenote
```

## Checks before a pull request

Run these from the repo root. On Ubuntu, keep the two `export` lines from the first build.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings
cargo test --locked -p sidenote --lib core::notes::    # the module you changed
python3 -I tools/prove_linux_boundaries.py . --self-test
cargo check --workspace --locked --tests                           # Windows only
cargo check --workspace --locked --tests --features asr-runtime    # Windows only
```

In PowerShell, quote Cargo's separator (`'--'`) and use `python` for `python3`.

The full test suite runs in CI. To run it locally, copy the temporary folders and
environment from the "Test default and ASR feature sets" step in `.github/workflows/ci.yml`.

`tools/prove_linux_boundaries.py` keeps a reviewed list of source files that start
processes or read files (`tools/linux-boundary-inventory.json`). If it reports a stale
inventory, list the process and file-read calls you added or changed in your pull
request; a maintainer reviews them and updates the inventory.

If you change `relay/` (with Bun 1.4.4):

```sh
cd relay
bun install --frozen-lockfile
bun run typecheck
bun test tests
```

If you change `tools/` or `scripts/` (the second line needs PowerShell 7, `pwsh`):

```sh
python3 -I tools/pipeline-tests.py -k test_routing. -k test_job_contract. -k test_fixed_patterns -k test_timeout_and_missing
pwsh ./tools/dependency_notices_proof.ps1
```

## How a pull request is checked

- CI runs on GitHub-hosted runners. One required check, **CI Gate**, must pass before merge.
- Jobs run by what changed: Rust code runs the Linux job; `relay/` runs the relay job;
  `tools/` and `scripts/` run the tools job; Windows-specific code can also run a Windows job.
  Changes under `.github/` run everything.
- Pull requests from people outside the project wait until a maintainer approves the
  CI run. This is a GitHub safety setting, not a judgement on your change.
- Your branch does not need to be up to date with `main` to merge.
- In the pull request, say what changed, why, and how you checked it. Add screenshots
  for UI changes, made with synthetic data.

## Licence of contributions

Sidenote is licensed under AGPL-3.0-or-later. By opening a pull request you agree that
your contribution is licensed under the same terms. There is no extra agreement to sign.
The maintainer may change this policy for future contributions; any change will be
written here first.

## Security and conduct

Do not open a public issue for a vulnerability; follow [SECURITY.md](SECURITY.md).
This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
