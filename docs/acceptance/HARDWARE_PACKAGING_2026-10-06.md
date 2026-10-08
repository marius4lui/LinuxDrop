# Hardware and package acceptance — 2026-10-06

Environment: dedicated Ubuntu 24.04 WSL2 distribution `LinuxDrop-Dev`, amd64, virtual Ethernet only. Existing host internet and radios were not changed.

| Check | Observed result |
|---|---|
| Hardware/netd unit tests | 4 passed: regulatory channels; protected preference exclusion; unknown helper operations; process start identity |
| Hardware/netd clippy | Passed with all targets and warnings denied |
| Passive inventory | Real `eth0` default route; no physical radios or Bluetooth; explicit missing NetworkManager/BlueZ diagnostics |
| Desktop file | `desktop-file-validate` passed |
| AppStream | `appstreamcli validate --no-net` passed (two pedantic notices) |
| Native systemd units | `systemd-analyze verify` passed after installation |
| Nautilus extension | Python compilation passed; actual file-manager menu interaction remains separate |
| DEB initial install | Passed with apt, with recommends disabled to avoid introducing network managers to the test guest |
| DEB same-version upgrade | Passed via dpkg; helper stopped/restarted |
| DEB remove | Binary, extension, unit and helper socket absent afterward |
| DEB reinstall | Passed; system helper active under dedicated `linuxdrop-netd` UID, mode-0600 lease journal |
| Installed helper negative paths | Actual Unix socket rejected an unknown execute operation and a 4097-byte request; service remained active |
| Source/licensing | Matching local source archive built alongside DEB; root and both protocol licenses installed |
| Windows launcher | Initial nested compositor rendered internally but the actual Windows window was black. App mode was changed to direct WSLg Wayland + Cairo + private D-Bus, with real daemon and explicit local demo peer |
| Windows rendering repair | Direct GTK rendered correctly. Root cause then proved: WSLg XWayland UID1000 could not attach UID1001 shared-memory buffers. Full nested GNOME with existing ubuntu UID 1000 and isolated ports 53327/53329 rendered correctly and the user confirmed the live demo works |
| Native shell extension | ACTIVE in the nested session; the final ubuntu demo uses the package-global extension |
| Screenshot | `installed-desktop.png` is an internal Wayland screenshot and does not validate Windows-visible rendering; the initial black-window failure was only caught by Windows capture |
| Reverse-download HTTP | Two tests passed: PIN/session authorization, exact concurrent 150KB transfers, refresh, invalid-session denial, shutdown; repeated invalid PIN rate limit |

The initial package cycle used debug binaries while implementation continued. A later release-binary DEB was also built and installed for the live demo. Final artifacts must be rebuilt after the remaining source changes. The nested GNOME session initially opened Overview; the launcher closes Overview through its public read/write D-Bus property. WSLg's shared X11 socket has unsuitable mode for an inner XWayland server, so that experimental session uses `--no-x11`. Software llvmpipe alone did not repair the outer black window; direct GTK Wayland+Cairo was launched as the immediate live-app path. Actual Windows-window evidence takes precedence over inner compositor captures.

Not verified without physical radios/devices: monitor/injection behavior, AWDL radio recovery after crash/unplug, BLE, Wi-Fi Direct, Apple/Android device interoperability. VM protocol tests and install success are not substitutes for those checks. Authenticated netd radio acquisition also needs an ordinary active local login session and suitable idle hardware; denied or unavailable hardware remains explicitly reported.
