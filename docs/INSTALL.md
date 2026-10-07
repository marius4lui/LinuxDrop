# Install and run LinuxDrop

## Ubuntu 24.04 desktop

The first native package targets Ubuntu 24.04, GTK 4.14/libadwaita 1.5, GNOME Shell 46, and amd64. Download or build the matching `.deb` and keep its accompanying source archive.

```sh
sudo apt install ./linuxdrop_0.1.0_amd64.deb
```

Open **LinuxDrop** from the application menu. Select files or drag them into the send area, select a nearby device and send. Visibility starts hidden; turn it on deliberately when receiving. Files are saved only after approval. Settings and wireless hardware have separate pages. Quick Share and Nearby Share refer to the same protocol backend.

The optional **LinuxDrop** GNOME extension provides the desktop bubble and Quick Settings. Enable it in the **Extensions** app. If installed while GNOME is already running, log out and back in so the shell discovers it. The bubble is hidden at startup. Click the LinuxDrop icon in the top panel to open it, then choose **Drop files** or hover a file drag over the open bubble. This opens an aligned native GTK drop surface; the shell anchors it while GTK receives the actual file payload. This handoff was exercised with Nautilus 46 local files on GNOME 46.2 Wayland. Sandboxed portal sources, simultaneous multi-selection, fractional scaling and physical multimonitor acceptance remain separate; other Shell versions need their own validation. The main GTK window is also a file drop target.

The package installs the app and user daemon, the restricted system radio helper, the AWDL link helper, the extension, and file-manager actions. The user daemon activates through session D-Bus when needed. The system helper runs as a dedicated account with network capabilities; the app never needs root. Administrator authorization is requested only for a radio lease. Removing the package stops/removes its system service and extension files; downloaded files and user preferences remain.

```sh
sudo apt remove linuxdrop
```

**Hardware:** LocalSend and Quick Share LAN can work over Ethernet or virtual Ethernet. BLE, Wi-Fi Direct, and AirDrop require real suitable radio hardware exposed to Linux. AirDrop additionally requires a dedicated idle adapter, usable channels, and experimental interoperability checks against your actual Apple devices. An empty wireless list in a VM accurately means that the guest has no wireless adapter; it does not block LAN sharing or the desktop interface. USB passthrough and a separate USB adapter can be tested later without changing the host's internet adapter.

**File managers:** Nautilus gains “Send with LinuxDrop” after a restart when `python3-nautilus` is installed. Dolphin's action is included. Thunar users can add the provided action through its custom-action editor; `/usr/share/doc/linuxdrop/thunar-uca.xml.example` is the reference and existing custom actions are preserved.

## This Windows development machine

A dedicated Ubuntu 24.04 WSL2 distribution named **LinuxDrop-Dev** has been created. Existing distributions and projects are separate. The full desktop demo uses its existing `ubuntu` user (UID 1000), matching WSLg's XWayland renderer. A separate direct-app demo uses `linuxdrop` (UID 1001). The compiler uses the root-owned `/opt/linuxdrop-target` build cache. The native package is installed in this distribution.

From the repository in PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File packaging/dev/Start-LinuxDrop.ps1 -Mode Desktop
```

This opens the installed app and GNOME Shell integration in a native desktop window, using a private D-Bus session and the real daemon. Actual Windows rendering was verified after matching the compositor UID to WSLg; an internal screenshot alone is insufficient. Starting the launcher again reuses the live desktop. No browser is used.

The optional **Demo-Empfänger (Ubuntu)** is a real loopback LocalSend endpoint, autoaccepting only this explicit demo. The desktop daemon uses port 53327 and its demo receiver 53329, isolated from the standalone app's 53317/53319. A sample file is `/home/ubuntu/LinuxDrop-Demo/Zum-Testen.txt`, with received bytes in `received` below that folder. `-Mode App` remains a direct GTK Wayland+Cairo fallback using the separate `linuxdrop` user. Closing an isolated demo ends its test services; an ordinary Ubuntu desktop uses its normal persistent user session.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File packaging/dev/Start-LinuxDrop.ps1 -Mode App
powershell -NoProfile -ExecutionPolicy Bypass -File packaging/dev/Start-LinuxDrop.ps1 -Mode Build
```

For a Windows desktop shortcut with the LinuxDrop icon:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File packaging/dev/Install-WindowsShortcut.ps1
```

The shortcut opens the verified Ubuntu desktop mode, with PowerShell hidden. Existing live windows are reused.

The WSL guest currently has virtual Ethernet, no Wi-Fi radio, and no Bluetooth controller. Multicast/LAN reachability depends on Windows/WSL networking and firewall settings; do not treat a successful loopback test as phone interoperability. The launcher does not alter network mode, host adapters, firewall rules, or existing distributions. AirDrop/Wi-Fi Direct/BLE acceptance is therefore pending physical hardware.

The experimental nested desktop uses `--no-x11` for inner clients because WSLg owns the shared outer X11 socket directory. A headless GNOME session used for automated screenshot review is separate and its internal screenshots do not prove that the outer Windows window rendered correctly.

## Build from source

Required tools are Rust stable (with rustfmt and clippy), GCC, pkg-config, GTK4/libadwaita development packages, OpenSSL, D-Bus/udev, libnl, libpcap/libev, and protoc. The setup script installs distro packages on an Ubuntu development VM:

```sh
sudo sh packaging/dev/setup-ubuntu.sh
# Install Rust as your normal development user via https://rustup.rs.
rustup component add rustfmt clippy
cargo build --locked --workspace
cargo test --locked --workspace
sh packaging/build-deb.sh
```

Outputs are `dist/linuxdrop_0.1.0_amd64.deb` and `dist/linuxdrop_0.1.0_source.tar.gz`. The source archive contains the actual source, vendor patches and lockfiles used for the local build. License texts and source provenance are also installed under `/usr/share/doc/linuxdrop/`. A local development build can include uncommitted source; do not confuse the recorded base Git revision with the accompanying complete source archive.

For a fresh environment, build the workspace and vendored filin helper before packaging. `CARGO_TARGET_DIR` is honored. A review package can use existing debug binaries with `LINUXDROP_SKIP_BUILD=1`, `LINUXDROP_TARGET_DIR=/path/to/target/debug` and `LINUXDROP_FILIN=/path/to/target/debug/filin`; release packages should use the default release build. RPM and Arch source recipes are available under `packaging/`. Their distro-specific build instructions and acceptance boundaries follow below.

## Fedora and Arch source packages

Build each package on its target distribution. The scripts perform release builds
of both the workspace and the AWDL helper and keep a matching source archive.
Do not copy Ubuntu executables into a distribution package for release.

On Fedora, install build requirements and run the RPM builder as your normal
build user:

```sh
sudo dnf install rpm-build systemd-rpm-macros cargo rust gcc gcc-c++ cmake \
  gtk4-devel libadwaita-devel openssl-devel dbus-devel systemd-devel \
  libnl3-devel libpcap-devel libev-devel protobuf-compiler glib2-devel
sh packaging/build-rpm.sh
sudo dnf install ./dist/linuxdrop-0.1.0-3.fc44.x86_64.rpm
```

The filename above is the Fedora 44 x86_64 build. Use the actual distribution and
architecture filename produced in `dist/`. The RPM recommends NetworkManager, BlueZ and `nautilus-python` for wireless,
Bluetooth and Nautilus integration. If installing without weak dependencies,
add the corresponding packages when enabling these features.

On Arch, install the declared build/runtime dependencies, then run `makepkg`
through the wrapper as a normal user (it intentionally refuses root):

```sh
sudo pacman -S --needed base-devel rust cmake pkgconf protobuf libnl libpcap \
  libev gtk4 libadwaita openssl systemd dbus polkit iw iproute2 ethtool python busybox
sh packaging/build-arch.sh
sudo pacman -U ./dist/linuxdrop-0.1.0-1-x86_64.pkg.tar.zst
```

Arch's optional runtime integrations are `networkmanager`, `bluez` and
`python-nautilus`. Its radio helper and child programs live in
`/usr/lib/linuxdrop`; the build embeds this directory and the package's systemd
unit uses the same path. Fedora and Debian use `/usr/libexec/linuxdrop`.
`LINUXDROP_LIBEXECDIR` is a build-time choice, never a runtime way to replace a
privileged child program. Prebuilt packaging checks must use binaries compiled
for the matching path; ordinary source builds configure it automatically.

Arch disables makepkg's C/C++ LTO flag for mixed crates such as `ring`; Cargo's
Rust thin LTO remains enabled, following the
[Arch Rust packaging guidance](https://wiki.archlinux.org/title/Rust_package_guidelines).

The RPM and Arch builds use AWS-LC's CMake builder to preserve distribution
hardening flags while applying the jitter entropy source's required per-file
optimization settings. They do not disable that entropy source. See the
[upstream build configuration](https://aws.github.io/aws-lc-rs/resources.html).

The GTK app requires GTK >= 4.12 and libadwaita >= 1.5. The bundled Shell extension
currently declares GNOME 46 and 50. Native actor/settings and installed GTK
drop-window checks passed on both; see [Shell acceptance](acceptance/SHELL_50_2026-10-07.md).
Other Shell majors require their own acceptance before being declared supported.

## Troubleshooting

- **No devices:** enable visibility on the receiving device, use the same permitted LAN, and inspect the backend status in LinuxDrop. No synthetic devices are shown.
- **No radios:** inspect Hardware. VM Ethernet is not a Wi-Fi adapter. Do not switch your current internet adapter into monitor mode.
- **AirDrop permission denied:** use an active local desktop session with a working polkit authentication agent. Remote/absent sessions cannot acquire radios.
- **Radio helper not running:** inspect `systemctl status linuxdrop-netd.service`. The package starts this service; source-only GUI launches do not install it.
- **Bubble missing:** click the LinuxDrop top-panel icon; the bubble is deliberately hidden until opened. If the icon is missing, enable the extension on GNOME 46 or 50. A desktop restart after package installation may be needed. Other desktops still run the native app.
- **Window closes but app behavior is unclear:** on a normal Ubuntu session the background daemon has its own D-Bus lifecycle. The isolated WSL launcher intentionally ties its test services to the nested desktop lifetime.

See [hardware and packaging design](HARDWARE_AND_PACKAGING.md) and the acceptance records for verified checks and outstanding device tests.
