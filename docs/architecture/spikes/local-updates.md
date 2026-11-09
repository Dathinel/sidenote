# Local update proof

The owner keeps this lane local. There is no remote update source, repository
URL, hosted release workflow or upload. Velopack remains the distribution choice.

The first N to N+1 run used Velopack Rust and CLI 0.0.1298 with .NET SDK 8.0.427.
The full N+1 package was 10,622,674 bytes. Its 1,527,891-byte delta reconstructed
a 13,579,769-byte full ZIP. Velopack accepted the reconstruction, but its bytes
did not match the published full archive's size and SHA256. The app correctly
refused our additional pre-apply checksum check.

Local clients therefore copy the full package, including when the feed offers a
delta. This keeps the feed's full-package checksum valid through download, cache
reuse and apply. A rejected cached file is set aside before retry. Packaging
still produces deltas. Supporting them later needs verification against extracted
file digests or an SDK API that verifies reconstructed contents independently of
the archive's compression. The cost today is a full local copy of about 10.6 MB
instead of a 1.5 MB delta. No custom installer or updater replaces Velopack.

Pending-package lookup and checksum verification happen before the recording
probe. A launch without a pending package does not wait on Hex to discover that
there is nothing to apply. MCP, listener and daily-check entry points do not run
the update service or this apply path.

The isolated evidence lives under `E:/Personal-Projects/reviews/updates-proof`.
The two release versions are proof-only overrides, not committed app versions.

The installed proof also showed that Velopack kills app-root processes again
after the fast after-update callback. Starting the listener in that callback
returned success, then Velopack killed it. The fast hook now preserves the resume
marker. The restart hook starts the validated task after final process cleanup.
The listener's OS-held singleton lock still rejects concurrent starts.
