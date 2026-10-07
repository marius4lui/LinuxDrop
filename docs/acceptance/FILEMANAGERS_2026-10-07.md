# Native file-manager selection acceptance

Fedora 44 private root, Nautilus 50.3.1, Thunar 4.20.9, Dolphin 26.08.1,
installed native LinuxDrop RPM revision 3, current Nautilus provider and new
Thunar Send To descriptor staged into their normal system locations.

Actual menu selection in all three managers opened LinuxDrop with exactly three
send-ready files. The selection includes spaces, an apostrophe, ampersand,
semicolon, literal command-substitution text, a leading hyphen and German Unicode.
AT-SPI verified all three exact removal-action labels and the ready draft count;
file contents were unchanged and the command-substitution canary was not created.

- Nautilus: native context-menu provider -> argv subprocess -> GTK draft.
- Thunar: native Send To submenu -> installed desktop entry -> GTK draft.
- Dolphin: native context-menu service -> KDE desktop-command expansion -> GTK draft.

The new Thunar descriptor is installed under `/usr/share/Thunar/sendto/` by the
shared package installer and is listed in the RPM payload. No user uca.xml edit
or terminal command is required. The existing optional custom-action installer
is retained. Thunar's MIME filtering uses exact equality (unlike Dolphin's base
MIME inheritance); a generic octet-stream filter hid ordinary text files, so the
Send To entry leaves filtering to LinuxDrop's existing regular-file validation.
This means Thunar can offer the entry for a folder, which the app then rejects.
The Nautilus menu now follows German/English desktop language priority.

Primary references: [Thunar Send To](https://docs.xfce.org/xfce/thunar/send-to),
[Thunar matching implementation](https://raw.githubusercontent.com/xfce-mirror/thunar/master/thunar/thunar-sendto-model.c),
[Dolphin service menus](https://develop.kde.org/docs/apps/dolphin/service-menus/).

## Reproduction and boundaries

`tests/integration/run-filemanager-selection.sh nautilus|thunar|dolphin` runs
unprivileged with private XDG state, bus, Xvfb and the existing daemon snapshot
fixture. It requires the installed manager/integration, pyatspi, xdotool, xprop
and openbox. The Fedora root additionally had private PID/mount/IPC/network
namespaces and system D-Bus. No protocol listener, host device mount, browser
or live desktop was used.

The native test checks actual menu actions, not a direct call to the plugin's
callback. Nautilus 50's GTK menu items expose empty accessible names here; the
fixture checks the observed 12-item menu shape before choosing its penultimate
item. Dolphin uses the observed default-layout file context menu and keyboard
navigation. These bounded native fixtures may require adjustment for changed
upstream menus; they are not universal desktop automation APIs.

Logs are in `dist/filemanagers/`. Menu and draft captures are in
[ui/filemanagers](ui/filemanagers). The draft captures were visually inspected.
They expose an additional Fedora GTK 4.22/libadwaita 1.9 layout issue in the
installed revision 3 app: content extends beyond its default 610-pixel window.
Selection correctness is established; native Fedora compact layout acceptance
remains open and needs the current app build plus a targeted layout diagnosis.
The revision 3 executable predates the subsequent relative-font/compact-layout
source fix. Do not infer visual completion from the AT-SPI selection assertions.

The private root has no FUSE device, so its portal mount warning is expected;
this test uses local files and does not claim a Flatpak/document-portal journey.
