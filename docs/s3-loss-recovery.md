# Recover a Lockbox vault after losing S3 access

Use this runbook when the original bucket is unavailable and the team needs a
working replacement vault. It uses an encrypted local `.lbx` snapshot and a new
Storj bucket. The original bucket can stay offline throughout the procedure.

If you can recover access to the original bucket and need to keep the same space,
restore provider access first. For a lost owner controller in an accessible,
enrolled bucket, see [Recover from Storj](cloud-recovery.txt). If the original
bucket is unavailable and keeping the space ID, roster, or signed history is
essential, review the [controller recovery API](sync-control-recovery.md#lost-controller-recovery)
before committing to a replacement. That path requires owner root authority,
verified surviving content and dependencies, and an independently authenticated
control and data anchor. The desktop procedure below creates a separate space.

## 1. Preserve and choose a local snapshot

1. Ask participants to pause changes in the old vault and preserve encrypted
   copies from trusted devices. Keep the latest `.lbx` and any `.lbx.bak` as
   separate files. Lockbox's automatic `.bak` is the previous saved file, and
   shared profiles can save several times during a sync cycle. Keep the originals
   in private storage. Work from a copy ending in `.lbx` so the file picker can
   select it, for example `incident-source.lbx`.
2. On a trusted machine with the old endpoint unreachable, inspect the working
   copy using **Open vault…**. A rollback or missing installation witness can
   make a shared copy read-only. That is sufficient for review. Keep rollback
   protection in place and preserve the source for history and later comparison.
3. Choose the snapshot based on verified content and known recent edits. A saved
   offline edit can exist on just one device. Compare the latest known edits, live
   entry and folder counts, and any missing items across available copies. Record
   which snapshot you picked, when it was captured if known, and what edits may
   be missing. If trusted copies disagree, reconcile changes record by record.
   Bulk importing a second snapshot can add conflicting values as separate entries
   or bring back entries deleted in the chosen source.

Bulk import requires the passphrase that protected the selected file when
it was saved. If you have only recovery words, the file's **Recovery** action can
open a snapshot with a recovery slot. A read-only shared backup keeps its original
password, which the bulk importer still requires. Preserve the readable backup
and manually copy approved records into the new vault if needed. See
[recovery words](recovery-words.md) and
[profile protection](profile-protection.md#commit-and-reconciliation).

If all local copies fail to open, get another trusted participant's copy or
prioritize regaining original bucket access. A replacement becomes authoritative
after its recovered content passes the verification below.

## 2. Create and populate the replacement vault

1. Provision a fresh private Storj bucket in an account the team controls. Give
   the owner an independent, bucket-scoped S3 credential with Read, Write and
   List permissions. The owner needs these permissions for the ordinary space
   objects and the `recovery/v1/` prefix. Save the gateway endpoint, bucket name
   and credentials securely. Use the gateway origin as **Base URL**, with the
   bucket in its own field. Lockbox's documented cloud setup targets Storj.
   [Lockbox Storj setup](storj-setup.en.txt) · [Storj credentials](https://storj.dev/dcs/api/s3/credentials)
2. Choose **Create vault…** and complete the identity prompt. Select **Multiple
   users**, enter the new bucket and credentials, and set a passphrase for
   the replacement. Choose a distinct vault name so participants can identify it.
   The desktop may reuse the owner's saved identity while creating a fresh space
   and device. If the owner's root key or recovery words were exposed, set up a
   fresh owner identity on a clean installation before creating the replacement.
   Keep the replacement passphrase and its recovery words available to the
   owner.
3. Open **Settings → Import vault** in the replacement. Select the working
   `.lbx` copy, enter that snapshot's passphrase, and choose **Import**.
   Lockbox reads the source locally and saves batches as ordinary signed changes
   in the replacement. The new bucket receives encrypted sync objects. Keep the
   `.lbx` files in private backup storage.
4. Review the import result. If import stops partway, fix the reported failure and
   retry the same source into the same replacement. Saved batches remain, and
   identical entries in the same mapped folder are skipped. If repeated retries
   fail, keep the source and the partly populated replacement for investigation.

If a replacement bucket is also unavailable, use the same import in a **Single
user** vault as an interim measure. Assign one operator to make any urgent edits
there. When cloud setup is available, use the verified interim `.lbx` as the
import source so those edits carry over. Preserve the original encrypted copies.

The import copies live entries, folders, notes, TOTP configuration, and attached
file bytes. It assigns new record IDs and can skip identical entries or merge
equivalent sibling folders. Old history, Trash, unresolved conflict versions,
excluded edits, memberships and invitations stay with the old space. Review any
of those items needed for operations in the readable source and explicitly
recreate the approved content. Edits absent from all surviving copies remain
unrecoverable through this procedure.

## 3. Verify and switch the team over

1. Reconcile the import counts and duplicate skips against the chosen snapshot.
   Check critical entries, their folder locations, TOTP configuration and
   representative attachment contents inside Lockbox. Resolve discrepancies
   before switching the team over. Keep passwords, recovery words and TOTP
   secrets out of incident records.
2. Wait for **Storj sync complete** on the owner profile. For a recovery-enabled
   controller this includes acknowledgement of the current recovery capsule.
   Give an approved participant separate credentials for the replacement bucket,
   issue a fresh invitation, and verify the participant's device and role through
   the usual trusted channel. On that independent device, confirm that synced
   records and an attachment are readable. If testing edit access, use a clearly
   identified test record and confirm the edit reaches the owner.
3. Give the remaining participants fresh invitations and confirm intended roles.
   Announce the replacement as the single vault for new edits. If the old bucket
   comes back, compare any missing data offline before explicitly copying it.
   If vault secrets may have been exposed, rotate the affected passwords, tokens
   and TOTP enrollments at their source systems and update the replacement.
4. Save an independent encrypted copy of the replacement `.lbx` outside the new
   bucket. Verify it opens using the saved snapshot password. Keep recovery words
   separately. Record the selected recovery point, known gaps, and when the first
   independent participant verified the replacement.

## Rehearse with fictional data

Use disposable buckets, an owner and a second installation. Create a source with
folders, an attachment, a TOTP entry, Trash and at least one saved offline edit.
Keep the old bucket unreachable, copy the source, and run the procedure. Verify
the offline edit appears in the replacement and the Trash item stays in the
source. Repeat the import and confirm an unchanged snapshot adds no entries.
Record how much data was missing at the chosen recovery point. Measure from the
start of the simulated outage until the second installation reads verified data.
Repeat with a snapshot that opens read-only to exercise the separate import path.

The existing [live Storj probe](sync-live-test.md) checks provider exchange and
admission. The recovery drill above exercises the snapshot-to-replacement path.
