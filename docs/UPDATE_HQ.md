---
audience: owner
order: 25
summary: "Update HQ on your phone, a computer or a server, and what to do before you do."
updated: 2026-11-19
---

# Update HQ

Each way of running HQ updates in its own way. Your Vault and settings are never touched by an update.

## On an Android phone

1. HQ tells you when a new version is ready: a notification, and a dot on Settings.
2. Open **Settings**, then **Updates**, or tap the dot on the gear. The Updates page shows the version on your phone, the new version, how big the download is, and what is new.
3. If you are on mobile data, HQ asks before it uses any. On Wi-Fi it does not.
4. Tap **Update**, then confirm when Android asks. HQ checks the download first.

To look for one yourself, open **Settings**, then **Updates**, and tap **Check for updates**. Before a big change it is wise to **Save a copy of my Vault**, which the update card offers. [Backup and restore](BACKUP.md) covers backups from a computer.

## On a computer

Run `hq update --apply`. 

## On a server

A server updates itself from signed releases and rolls back if a check fails. See [How HQ updates](UPDATE_SYSTEM.md) for the schedule, how to pause it and how to block a version.

## If an update is withdrawn

Sometimes a release is pulled after it ships. HQ says so in a notification: open HQ to see what to do.
