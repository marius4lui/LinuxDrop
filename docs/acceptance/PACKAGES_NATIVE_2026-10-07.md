# Native distribution package acceptance - 2026-10-07

This pass builds LinuxDrop from source with each distribution's own Rust/C
compiler and libraries. Earlier prebuilt repackaging is not used as native-build
proof. Isolated official Fedora 44 and Arch container filesystems run inside
LinuxDrop-Dev with private mount/PID/IPC namespaces, no host device mounts, and
separate Cargo targets. Cargo runs serially with two jobs. Networked operations
are package dependency installation only; daemon/GUI checks use a separate
network namespace containing loopback and a disposable veth interface.
The user's installed Ubuntu demo is not changed or restarted.

## Packaging corrections

- RPM/Arch declare the GTK >= 4.12 and libadwaita >= 1.5 runtime minimums.
- Arch installs helper programs in `/usr/lib/linuxdrop`, its native package
  convention. The same path is embedded in netd at compile time and written to
  its service unit. Debian/Fedora retain `/usr/libexec/linuxdrop`. No runtime
  environment variable changes a privileged program path.
- Arch installs the project's license in its package license directory.
- Distro CFLAGS exposed an aws-lc-sys 0.45.0 jitterentropy build failure: its
  cc-rs flag probe caches optimization flags before the source-specific guard.
  Both recipes select the upstream CMake builder, whose per-source options
  enforce the required unoptimized entropy build. No entropy or assembly source
  is disabled, and no blanket removal of distribution hardening flags is used.
- Arch's default C/C++ LTO produced unresolved `ring` symbols under Rust's lld.
  `options=('!lto')` removes makepkg's cross-language LTO injection; the project
  retains Cargo's release thin LTO. This follows the
  [Arch Rust packaging guidance](https://wiki.archlinux.org/title/Rust_package_guidelines).
  The AWS-LC builder setting is described in its
  [upstream configuration](https://aws.github.io/aws-lc-rs/resources.html).
- RPM revision 2 recommends NetworkManager, BlueZ and nautilus-python, matching
  the optional integrations provided by the other package formats.

## Fedora 44 x86_64

Native tools: Rust/Cargo 1.98.1, GCC 16.2.1, CMake 4.3.0, GTK 4.22.5,
libadwaita 1.9.4, glibc 2.43, systemd 259.9.

The workspace and vendored `filin-rs` release builds passed. A fresh RPM
installation was tested after removing the old fixture's helper account.
The package recreated the restricted user/group with `/nonexistent` home and
`/usr/sbin/nologin` shell. Package verification, dynamic libraries for all four
binaries, desktop-file validation, schema compilation and static systemd unit
verification passed. The installed app and AWDL helper also accepted `--help`.

The installed daemon passed `tests/integration/daemon_session.py` over real
private D-Bus and HTTPS: acceptance/rejection, selected files/custom destination,
restart admission, stored settings/history, diagnostics, download-link policy,
reverse-download PIN/consent and shutdown. This is a software protocol fixture,
not physical Android/Apple interoperability.

Same-version reinstall/removal passed. An actual RPM revision upgrade from
`0.1.0-1.fc44` to `0.1.0-2.fc44` then passed using normal DNF weak dependencies.
NetworkManager, BlueZ and Nautilus bindings were installed. A valid user-owned
preference file remained byte-identical across upgrade and removal. The four
binaries remained byte-identical to the fully checked revision-1 build. Package
payloads, service units and extension files were absent after removal.

The installed GTK app was also launched through the installed desktop file as
an unprivileged user under a private D-Bus/Xvfb session. AT-SPI confirmed that a
filename containing spaces and an ampersand reached the visible draft. Settings
and Hardware CLI requests reused the same app bus owner; returning to Send
retained that draft. The repeat after upgrading to RPM revision 2 passed too.
The saved check is `tests/integration/installed_gtk.py`; it refuses root and an
already-owned LinuxDrop bus name. This checks accessible UI state, not spoken
screen-reader output. A Gio.DesktopAppInfo deprecation warning is fixture noise.

Artifacts are in `dist/fedora-44/` with exact source archives, build/install/
upgrade logs, scripts and SHA256SUMS. Current install artifact:
`linuxdrop-0.1.0-2.fc44.x86_64.rpm`.

- RPM SHA256: `58c9bfbe131303cb8f9acc8769c45d74a9d31f57b37a874268e1c9773db80dd1`
- Matching source SHA256: `1e6ee034e067e7e45206a2d2177beb984640b2750caca502da30fbdac22f3d71`

## Arch x86_64

Native tools: Rust/Cargo 1.99.0, GCC 16.2.1, CMake 4.4.4, GTK 4.22.5,
libadwaita 1.9.4, glibc 2.44, systemd 262. Root image version 20261004.0.606936,
packages updated for this check. `makepkg` runs as the unprivileged builder user.
A private builder-owned Cargo cache avoids changing host cache permissions.

Both workspace and separate helper release builds passed after the documented
LTO correction. Fresh installation created the restricted helper account.
`pacman -Qkk` reported 77 files and zero altered files both after installation
and after reinstall. All four executable dependencies resolved. The installed
netd contains the `/usr/lib/linuxdrop` path and not the old libexec path; its
unit and child programs agree. Desktop entry, schemas, license and static unit
validation passed. The installed daemon passed the same private D-Bus/HTTPS
scenario as Fedora, and the installed GTK app passed the same unprivileged
AT-SPI desktop-launch/navigation/draft check. Reinstall and removal passed,
including removal of package-owned units, helpers, extension and license files.

The base container deliberately used pacman's NoExtract for all documentation;
that fixture rule initially hid package documentation. It was removed in this
isolated root before repeating installed-payload verification. The package
archive itself already contained the documentation; no product exclusion or
verification bypass was added.

Artifacts, source, scripts, logs and verified SHA256SUMS are in `dist/arch/`.
The debug package is optional. Install artifact:
`linuxdrop-0.1.0-1-x86_64.pkg.tar.zst`.

- Package SHA256: `58753fe54ef964f52572f315f9e9ba0bff17d9c89b9f6cb184c9120fce8d8f37`
- Matching source SHA256: `1e6ee034e067e7e45206a2d2177beb984640b2750caca502da30fbdac22f3d71`

## Remaining acceptance

These are container-root package checks, not booted distribution desktops.
Static service verification does not prove service startup/restart on a fully
booted Fedora/Arch system, active-seat polkit interaction, or suspend recovery.
Arch same-version reinstall is not a cross-version upgrade acceptance.
The extension still advertises GNOME 46 only; newer Shell compatibility has not
been established by packaging it. Full file-manager UI selection, Flatpak
chooser/host-service integration, screen-reader speech and physical mixed-DPI
acceptance remain separate software/desktop work. No new physical radio, Apple
or Android transfer evidence is claimed. These boundaries keep the overall
LinuxDrop completion goal open.

The subsequent [GNOME 50 packaging follow-up](SHELL_50_2026-10-07.md) supersedes
the revision-2 Fedora/revision-1 Arch artifacts above with Fedora revision 3
and Arch revision 2. It also proves an actual Arch revision-1 to revision-2
upgrade with retained user preferences and declares the tested 46/50 Shell
versions. Earlier package results and hashes above remain historical evidence.
