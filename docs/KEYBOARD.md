# Keyboard operation

Open the keyboard button in the application's header, or press Ctrl+?, for the
native shortcut guide. The guide scrolls in a compact window and never replaces
an active consent dialog.

| Shortcut | Action |
| --- | --- |
| Ctrl+O | Choose files |
| Ctrl+1 | Open the Send view; does not send files |
| Ctrl+2 | Open Transfers |
| Ctrl+3 | Open Hardware |
| Ctrl+4 or Ctrl+, | Open Settings |
| Ctrl+? | Show keyboard shortcuts |
| Tab / Shift+Tab | Next / previous control |
| Space | Activate the focused button or toggle |
| Escape | Close a dismissible dialog or the expanded Notch |
| Ctrl+Q | Close app windows, following the configured close behavior |

File selection, sending and incoming acceptance remain separate explicit steps.
Switching views never accepts or sends a transfer. Closing follows Settings:
background sharing can remain active, or the daemon can exit once idle.

The GNOME panel button opens the Notch. Its optional desktop-wide shortcut is
configured separately in Notch preferences; it is disabled by default and reports
conflicts with existing desktop shortcuts. Previous/next transfer buttons are
keyboard focusable. A selected transfer stays selected across progress updates,
unrelated transfers and completion while the Notch is open. A different consent
request does not inherit focus on an acceptance button. Closing the Notch resets
the selection for the next opening.

GTK uses native toggle, progress and form controls for state exposure. Symbol
actions have translated accessible labels, with file/device context where
needed. GNOME progress exposes its percentage in the accessible name. This
documents implemented behavior; full traversal with a real screen reader remains
an explicit acceptance item.
