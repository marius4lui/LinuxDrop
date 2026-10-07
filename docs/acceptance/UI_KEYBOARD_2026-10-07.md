# Keyboard help and transfer completion

Added a native, translated shortcut guide reachable from the header and Ctrl+?.
Ctrl+1 through Ctrl+4 select the four views through the existing window page
action. Ctrl+O, Ctrl+, and Ctrl+Q retain their existing behavior. The guide does
not stack dialogs or cover a currently open consent dialog. Manage Devices and
Refresh Nearby now carry explicit translated accessible labels as well as
tooltips; other custom symbol actions already have contextual labels in source.

The native GTK scenario checks the compact guide, repeated invocation, shortcut
registration and page action. Its German 480x600 render was inspected: labels
wrap and the native preferences page scrolls. Test captures disable animations
and wait after page changes to avoid sampling transitional or invalidated render
nodes; production animation preferences are unchanged.

GNOME 46/Mutter 46.2, German, 800x600 and 150% text: the existing complete native
scenario passes with new completion checks. Reordered snapshots retain the
selected completed transfer and Done-button focus. Completed incoming files
expose Open Folder and Done, with no stale Cancel action. Existing checks cover
active-transfer navigation, unrelated arrivals/departures, progress focus and
error recovery. The folder handler's GIO path-to-parent URI launch is source
reviewed; this scenario does not launch a real file manager.

Application all-target Clippy passed. These checks do not claim real screen-reader
traversal, fractional monitor scaling, or physical mixed-DPI acceptance.

User-facing shortcut reference: [Keyboard operation](../KEYBOARD.md).
