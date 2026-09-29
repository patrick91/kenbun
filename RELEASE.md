---
release type: patch
---

This release reads repository files written by common Windows tools.

- Decode text that starts with a UTF-8, UTF-16LE, or UTF-16BE byte order mark.
  PowerShell's `pip freeze > requirements.txt` writes UTF-16LE, which
  previously made scans partial, and a UTF-8 BOM hid the first requirement.
- Content without a byte order mark must still be valid UTF-8.
