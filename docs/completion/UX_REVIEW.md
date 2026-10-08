# Focused UX review — 2026-10-06

Scope: static inspection of the GTK app and GNOME extension, plus the existing
`docs/acceptance/ui/incoming.png`. No browser, live-demo interaction, process
restart, or source edits. Source is being updated concurrently; findings describe
the inspected implementation, not a post-fix acceptance result. The desktop owner
has acknowledged findings 1–3 and is implementing them.

## Top three fixes

1. **P1 — Retain files added while sending starts.** Code-confirmed:
   `app/linuxdrop/src/ui.rs`, `Ui::start_send`, captures the submitted paths before
   asynchronous calls, then clears the entire current file list on success.
   Choose/add/drop/remove remain available. Adding B while A is preparing silently
   removes B even though only A was submitted. Remove only submitted file identities
   or explicitly freeze editing. **Accept:** delay PrepareSend/StartSend, submit A,
   add B, complete A; B remains ready in the draft. Failed submission retains A.

2. **P1 — Preserve keyboard focus during updates.** Code-confirmed:
   `Ui::refresh` calls `render_peers` and `render_transfers` for every new snapshot
   revision; both clear their containers. Progress on one transfer therefore
   destroys unrelated focused peer/action widgets. `render_files` also clears
   all rows after each asynchronous metadata result. Use stable keyed rows and
   update values, or restore logical focus with a defined fallback if an action
   disappears. **Accept:** hold focus on a peer, Cancel, and Remove respectively
   while unrelated progress/metadata updates arrive; focus stays on that action
   and Return still targets the same item. Screen-reader behavior needs actual
   assistive-technology acceptance.

3. **P1 — Bind protocol choice to an ID.** Code-confirmed:
   `Ui::render_peers` builds the dropdown from the clicked peer snapshot;
   `Ui::start_send` resolves its numeric selection against the latest snapshot.
   Reordering protocols can send using a different protocol from the visible
   choice. Preserve the selected protocol ID and reconcile displayed options.
   **Accept:** select Quick Share, reorder/remove backend entries, then send;
   Quick Share remains selected or becomes explicitly unavailable, never silently
   mapped to another protocol.

## Further actionable findings

4. **P2 — Preserve unsaved settings edits.** Code-confirmed:
   `app/linuxdrop/src/settings.rs::render` clears every settings widget when any
   config value changes. Type a new device name without applying, then change
   Appearance: rebuilding uses the persisted name and loses the typed text.
   Retain dirty entry buffers and focus, or update rows in place. **Accept:**
   unapplied text survives another setting update and a service resync; Apply
   persists the intended text. Search text/category already survive reconstruction.

5. **P2 — Make incoming consent informative.** Code-confirmed:
   `Ui::render_transfers` shows peer, filenames and protocol before Accept, but
   displays total size only once transferring/completed and gives no destination
   preview or per-file controls. Show count/total size and actual destination
   before acceptance; expose the planned per-request destination/subset contract.
   **Accept:** a multi-file incoming request shows size and save location before
   accepting; changing location or excluding a file affects the accepted request.

6. **P2 — Do not claim invalid drops are ready.** Code-confirmed:
   `Ui::notch_drop` labels all added items “ready” immediately after `add_files`,
   before asynchronous validation completes. A folder or unreadable file gets the
   same success message. Use neutral “added” wording or a shared validation status
   with valid/invalid/checking counts. **Accept:** drop a regular file plus folder;
   the bubble never says both are ready, and review retains the valid file with a
   concrete reason for the invalid item.

7. **P2 — Explain link lifetime and dismissal.** Code-confirmed:
   `Ui::show_link` sets its close response to `stop`, so Escape also calls
   StopDownloadOffer after the user copies a link. The dialog does not explain
   that dismissing it revokes sharing. Either explicitly state “Keep this dialog
   open to share; closing stops sharing” or provide a persistent offer view with
   separate Close and Stop controls. **Accept:** the user can predict whether
   Escape ends the link; expiry and explicit stop produce visible status.

8. **P2 — Expose selection and contextual action names.** Code-confirmed:
   `Ui::render_peers` conveys selected peer only through CSS and a check image.
   Use supported toggle/selection semantics. `settings::render` creates history,
   desktop-preference and diagnostic icon buttons without explicit accessible
   labels; their parent row titles do not establish a verified button name.
   GTK progress also needs a transfer-specific name; Shell `_renderBody` uses
   width-only decorative progress widgets. **Accept:** accessibility inspection
   identifies selected peer, each icon action and the transfer associated with
   progress; confirm traversal/announcements with a real screen reader separately.

9. **P2 — Make history deletion recoverable or deliberate.** Code-confirmed:
   `settings::render` invokes ClearHistory immediately from the trash button;
   daemon `clear_history` removes terminal records and persists the result.
   There is no confirmation or undo. Add a concise confirmation or undo, state
   that received files remain, and give a completion indication. **Accept:**
   cancellation preserves history; confirmation clears terminal records, preserves
   active transfers and files, and communicates success.

## Verified improvements and boundaries

- Panel bubble starts hidden and opens through the panel button; this satisfies
  the latest interaction direction. No permanent floating notch is recommended.
- New asynchronous file validation retains per-file reasons, permits zero-byte
  regular files, and ignores results for removed files. Sending errors retain the
  draft. These are code facts, not portal/device acceptance.
- GTK reconnect installs signal/owner handlers on each proxy, clears revision on
  owner change, and retries a dirty refresh. No further high-confidence reconnect
  defect was found in this targeted pass; owner loss/recovery still merits one
  isolated acceptance scenario with a retained draft and selected peer.
- Settings query/category persistence and inclusion of desktop/diagnostic rows
  are already implemented; do not reintroduce them as missing features.
- The existing incoming screenshot has readable action placement but contains an
  English status inside German UI. Current i18n source now translates that exact
  status, so this is stale render evidence to refresh, not a current code defect.
- No current rendered, keyboard, screen-reader, physical-device or monitor-hotplug
  acceptance is asserted by this review.
