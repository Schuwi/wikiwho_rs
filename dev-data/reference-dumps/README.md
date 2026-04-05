# Wikimedia Reference Dumps

This directory is for committed upstream Wikimedia dump files used by development examples and exact-test coverage.

## Format

- Files in this directory are raw Wikimedia dump artifacts, not transformed fixtures
- The current dump is stored as compressed XML (`.xml.zst`)

## Licensing

Files committed here are Wikimedia-derived text data, not software covered by the repository's MPL/MIT code license.

For Wikimedia public text dumps, the relevant reuse guidance generally includes:

- Creative Commons Attribution-ShareAlike 4.0
- GNU Free Documentation License where applicable to the source project/content

Reusers should preserve attribution, link back to the source project or dump origin where practical, and indicate if they redistribute modified derivatives. Wikimedia also notes that dumps may contain fair-use material or unnoticed copyright violations, so reuse should follow the upstream guidance rather than treating these files as relicensed project assets.

## Attribution Practice

When committing or updating dump files here:

- keep this file in the directory
- update `ATTRIBUTION.md`
- record the dump filename, project, dump date, and upstream origin on `dumps.wikimedia.org`
- note whether the file is stored in raw upstream form or transformed further

Do not mix local-only scratch dumps into this directory.
