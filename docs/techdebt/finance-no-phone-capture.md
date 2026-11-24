# Only the phone reads messages, and only with READ_SMS

**Reason skipped:** the phone import needs the restricted `READ_SMS` permission, which Google Play limits. A person who cannot be granted it has no way in but `hq finance import <file>` on a computer. There is no file picker or share target, and notification and SMS monitoring are deliberately not built.
**Date:** 2026-11-21
**Where:** `apps/android/app/src/main/kotlin/example/hq/spike/finance/`, `apps/android/app/src/main/AndroidManifest.xml`
**Remove when:** a file picker or share target runs the same import on the device, or a Play release has its own review and build flavor for the permission.
