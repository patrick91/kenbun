---
release type: patch
---

Release-pipeline verification and fixes (no library changes since 0.2.0).

- Wheel builds no longer run autopub on every platform; the release version
  is computed once and stamped with a tomlkit-only script, so windows-arm
  (which has no cryptography wheel) builds cleanly.
- Fixed the post-publish git step so the GitHub release, tag, and changelog
  are created automatically.
