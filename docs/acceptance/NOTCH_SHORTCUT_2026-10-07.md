# Notch shortcut and monitor interaction

Notch preferences now include an optional native key-combination capture dialog.
It is disabled by default. Apply stores the combination; Escape cancels and
Backspace removes it. Tab/Return remain available for dialog navigation. Ordinary
typing keys and Shift-only combinations are rejected. The preferences display
active, disabled, unavailable/conflicting and extension-inactive states.

The extension uses Mutter's `grab_accelerator` result and leaves an occupied
combination untouched. A successful grab is allowed in the normal desktop and
Overview modes, never the lock screen. Reconfiguration and extension disable
release the owned grab. Primary references:
[grab_accelerator](https://mutter.gnome.org/meta/method.Display.grab_accelerator.html)
and [external binding name](https://mutter.gnome.org/meta/func.external_binding_name_for_action.html).

Monitor-layout changes close the bubble and any owned drop surface before
repositioning. Monitor indices can be reassigned after hotplug; merely checking
whether the old index still exists could move an active consent interaction to
another display. Reopening resolves the configured primary/pointer/fixed policy
again, including fallback for a missing fixed monitor. Ordinary pointer movement
continues to preserve the monitor selected at interaction start.

The isolated GNOME 46/Mutter 46.2 test passed in German at 800x600 with 150% text.
It exercises an actually occupied accelerator, conflict refusal, release/retry,
open/close signal dispatch, plain-key rejection and final release. Native
preferences exercise key capture, persisted assignment, removal, keyboard Tab
navigation and conflict feedback. Controlled layout-change notification closes
the interaction and allows reopening. This is not physical monitor hotplug or a
physical-keyboard input test. The existing transfer-selection, keyboard-focus,
verification, failure and owner-replacement scenarios pass in the same run.
