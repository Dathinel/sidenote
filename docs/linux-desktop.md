# Linux desktop launch

The `desktop` feature includes the shared screens and Rune UI dependencies. `ui-kit`
remains a compatibility alias. The pinned Rune revision supplies winit X11/Wayland,
AT-SPI accessibility and wgpu Vulkan; no Rune fork or vendored backend is used.

Build on Linux with Rust stable, a native C linker, libxkbcommon development files
and libxkbcommon-x11 for X11 sessions. Mesa Vulkan drivers provide software rendering
for Xvfb qualification:

```sh
RUSTUP_TOOLCHAIN=stable cargo build --no-default-features --features desktop --target x86_64-unknown-linux-gnu --bin sidenote
RUSTUP_TOOLCHAIN=stable cargo clippy --no-default-features --features desktop --target x86_64-unknown-linux-gnu -- -D warnings
```

The compositor/window manager places the Linux window; Rune's portable center request
is used where supported. The compositor supplies the window icon until Linux packaging.
Bundled, OFL-licensed Inter supplies Linux titles and body text. Windows keeps its
existing Georgia title loading, with Inter as the fallback when Georgia is unavailable.

An OS-held lock in an owner-checked 0700 user namespace excludes a second desktop
process for the same canonical data directory (SHA-256 key). XDG_RUNTIME_DIR is used
when set; otherwise the namespace is ~/.local/state/sidenote/desktop-locks. Ancestors
are pinned without symlinks and must be self/root-owned and protected from other
users replacing entries. Lock files are owner-checked regular 0600 files opened
without following symlinks. They are never unlinked while active.
A second launch exits with `SIDENOTE_ALREADY_OPEN`; focus forwarding is not implemented.
Closing flushes the shared draft state and quits. If saving fails, the window reopens
with its draft retained. The kernel releases the lock when the process exits or crashes.

Linux tray, automatic updates, Hex installation and recording control are unavailable
in these launch layers. Settings labels those limits. File/folder reveal uses xdg-open
only in a desktop session. Save dialogs and file clipboard actions remain unavailable.
The server worker containment and notes execution policies are unchanged.

Qualification must use an empty temporary `SIDENOTE_DATA_DIR`, explicit meetings and
peers directories, and only synthetic repo fixtures. `SIDENOTE_STATE_DIR` marks an
isolated UI fixture run and disables live Hex watchers; never point it at owner data.
