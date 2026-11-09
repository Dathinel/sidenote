# Shared standard licence texts

These texts are committed so notice generation does not need network access or a
particular cache layout. The packer copies each shared text once. Dependencies
whose published crate omits licence files receive a separate notice containing
their own locked Cargo name, version, authors (or repository) and complete licence
expression. Original upstream notice files are still copied without changes.

The initial texts came from this device's cached published crates:

| Text | Cached source | Extraction |
| --- | --- | --- |
| MIT | `sha2 0.11.1/LICENSE-MIT` | Unchanged grant and disclaimer, starting at `Permission is hereby granted`. The dependency notice supplies its own metadata attribution instead of borrowing RustCrypto's copyright lines. |
| Apache-2.0 | `sha2 0.11.1/LICENSE-APACHE` | Complete file, including the standard appendix. |
| BSL-1.0 | `error-code 3.4.0/LICENSE` | Complete Boost Software License 1.0 text. |
| Zlib | `zlib-rs 0.6.8/LICENSE` | Unchanged terms starting at `This software is provided`. The dependency notice supplies its own attribution instead of borrowing Trifecta's copyright line. |

Line endings are normalized to LF. MIT and Zlib have descriptive headings.
Unknown identifiers, exceptions, incomplete expressions and missing copyright
attribution remain uncovered. An expression containing an unsupported identifier
is not silently reduced to one of its alternatives. An owner-approved choice of a
standalone OR alternative can be recorded per name, version and exact expression
in `tools/license_choices.json`. The notice retains the full declaration and
records the selected alternative. A changed declaration invalidates the choice.
