# Unblock publication of the workspace and composer correction

Issue #516 was claimed before changes because master failed during Linux AppImage packaging, before tests or publication. The actual packager replay failed with HTTP 504 while fetching its runtime.

The installer workflow now resolves a pinned, checksum-verified runtime before invoking Tauri. The helper retries downloads, checks cached bytes, and installs valid downloads atomically. The actual AppImage output command succeeded against the same isolated staging directory when given the verified runtime; cached resolution also succeeded with a curl executable that refuses downloads.

The new corrupt-runtime regression passes and rejects fresh/cached invalid bytes without returning a path or retaining temporary downloads. Full release CI and post-merge publication remain required. This does not establish interactive Windows acceptance: Zero remains offline and the previous read-only capture task still needs cleanup when it reconnects. Latest published desktop at this writing is test.1030, which includes the workspace but predates the composer correction.
