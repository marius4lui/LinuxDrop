# Linux hardware and native packages

The initial build target is Ubuntu 24.04 (GTK 4.14, libadwaita 1.5, GNOME 46). The development environment is a separate `LinuxDrop-Dev` WSL2 distribution. It has virtual Ethernet and no passed-through wireless radio. Absence of physical radios is reported as absence; discovery does not invent peers or capabilities.

## Hardware contract

`linuxdrop_hardware::inventory()` collects physical radio identity from sysfs (including udev-populated USB/PCI properties), kernel-reported modes/channels through the `iw` nl80211 client, NetworkManager connections through system D-Bus, and BlueZ controller/advertising limits through ObjectManager. `watch_inventory()` reacts to udev net/radio/rfkill/Bluetooth events and reconciles every five seconds so service restarts and missed events heal automatically. Collection does not change interfaces, channels, NetworkManager profiles, or rfkill.

The inventory's observation timestamp and each radio's kernel/driver record qualify capability evidence. Monitor support is kernel-reported. Injection and AWDL remain explicitly unknown until device interoperability testing; the AR9271 name alone never establishes compatibility. A missing serial number remains missing; the physical device path is the fallback identity.

Selection applies exclusions before preference scores. Blocked radios, missing drivers, leased radios and active/default-route or connecting radios cannot be acquired. AWDL additionally requires reported monitor mode and a permitted non-DFS/non-no-IR social channel. Direct Wi-Fi requires a managed-station mode, an enabled channel and an idle interface; regulatory restrictions further constrain its hosting roles. An explicitly preferred adapter cannot override these restrictions. LAN transfers can use Ethernet. Multiple interfaces on one wiphy remain one radio.

AirDrop and Quick Share are planned jointly. The planner first maximizes the number of usable radio transports, preserving scarce AWDL capability for AirDrop when another station adapter can serve Quick Share. With one AWDL radio, AirDrop receives it while Quick Share retains LAN. Remaining ties use explicit preference, capabilities, USB preference and stable IDs. A successful cleanup after failed AirDrop startup makes that radio available for Direct Wi-Fi; an unconfirmed cleanup keeps it excluded.

Automatic USB use can be disabled without preventing an explicitly preferred USB adapter. Changing that setting uses the normal guarded backend restart. Hotplug applies to both protocols and waits for active transfers/download links to finish; the arrival is not forgotten while busy. Own temporary-interface churn cannot repeatedly trigger automatic acquisition attempts on the same selected attachment. Unplug/replug or rfkill resets that attempt budget, and an explicit restart remains available. Hardware details show the actual reservation holder; reserved does not imply backend readiness or proven interoperability.

## Restricted helper

`linuxdrop-netd` runs under its own system account with only `CAP_NET_ADMIN` and `CAP_NET_RAW`. It listens at `/run/linuxdrop/netd.sock`. The protocol uses one bounded JSON message per line, connection-owned leases, Unix peer credentials, the actual process start time, an active local logind session, and polkit action `io.github.marius4lui.LinuxDrop.manage-radio`. The GUI and file-transfer daemon stay unprivileged.

Operations include monitor/AWDL acquisition, direct-radio reservation, P2P group joining/hosting/leaving, channel selection, bounded active diagnostics, release, status and recovery. They accept inventory IDs and constrained values, not shell commands, executable paths, or arbitrary output files. Commands are fixed absolute binaries with argument arrays and a cleared environment. A held socket keeps the lease alive; disconnect releases it. The two-leases-per-connection and sixteen-client limits bound concurrent activity. New unauthenticated connections time out after ten seconds.

`acquire_awdl` adds a uniquely named monitor interface on an idle radio, launches the vendored `filin` link helper, and returns its unique nonpersistent TAP name. Filin receives only the radio's currently permitted, non-DFS, non-no-IR frequencies through `LINUXDROP_ALLOWED_FREQUENCIES`; the vendored patch enforces the allowlist at every channel mutation. AirDrop HTTP/TLS/mDNS run separately without privileges. Monitor frames and TAP access remain confined to the helper. No force-master mode or diagnostic HTTP server is enabled.

Before mutation a mode-0600 atomic journal records the lease. Recovery deletes only a same-boot, matching-phy VIF whose alias equals its lease marker. An ambiguous interface is left intact and reported for manual recovery. AWDL acquisition preserves original VIFs, NetworkManager managed state and saved connections. Direct Wi-Fi separately owns temporary connections/groups and removes only resources identified by its lease during restoration. A two-second watchdog stops the child on exit, unplug, or competing interface activation. systemd's control-group stop also terminates child helpers; kernel TAP lifetime follows the child file descriptor. Restoration after a real device crash/unplug remains a hardware acceptance item.

The currently permitted frequency set is captured at acquisition. The watchdog stops the lease if a previously granted frequency becomes restricted; clients then reacquire after the regulatory-country change. Kernel regulatory enforcement remains authoritative between inventory updates. There is no automatic regulatory-country override. A second inventory check after monitor-VIF creation catches connections that started during acquisition; later competing activation is detected by the two-second watchdog. Userspace snapshots cannot promise an atomic lock against another privileged network manager.

## Build and install

On Ubuntu 24.04, `sudo sh packaging/dev/setup-ubuntu.sh` installs native build dependencies. Install stable Rust for your normal user, then:

```sh
cargo build --workspace
cargo test -p linuxdrop-hardware -p linuxdrop-netd
sh packaging/build-deb.sh
sudo apt install ./dist/linuxdrop_0.1.0_amd64.deb
```

`CARGO_TARGET_DIR` is honored, and `LINUXDROP_TARGET_DIR`/`LINUXDROP_FILIN` can explicitly select prebuilt binaries. Set `LINUXDROP_SKIP_BUILD=1` only when those binaries have already been built. Package staging requires `/usr` because desktop and service files use installed absolute paths. The package creates the dedicated helper account, starts its system service, and provides session D-Bus activation for `linuxdropd`. Merely installing the package does not make the user visible or start the user daemon. User transfer files and configuration survive removal.

Enable the optional GNOME extension in Extensions under UUID `linuxdrop@marius4lui.github.io`. GNOME Shell 46 is the initial runtime target. The app's GApplication ID is `io.github.marius4lui.LinuxDrop.App`; the daemon independently owns `io.github.marius4lui.LinuxDrop`.

Nautilus gets a Python menu provider (restart Nautilus after installation); local regular file selections are passed as distinct subprocess arguments, including whitespace and Unicode. Dolphin gets a KIO service menu. Thunar's example custom action is installed under `/usr/share/doc/linuxdrop/thunar-uca.xml.example`; existing personal actions are never overwritten. Add it through Thunar's custom-action editor. RPM and Arch source recipes are included; they require distro-specific installation acceptance before release claims. The Arch source archive is local and must be checksum-pinned when publishing a release package.

Development scripts keep their work in the chosen VM: `packaging/dev/run.sh` starts a shared D-Bus daemon/app session, and `packaging/dev/nested-gnome.sh` starts a nested Wayland GNOME 46 session as a normal user. Normal desktop sessions use the installed D-Bus activation instead.

## Acceptance boundaries

Meaningful tests cover regulatory parsing, protected-radio selection, unknown-operation rejection, and process start identity. The WSL inventory confirms virtual Ethernet/default-route reporting and accurately reports missing NetworkManager/BlueZ services. Physical radio injection, polkit interaction from a normal graphical session, adaptive AWDL channel behavior, and crash restoration with actual USB hardware remain independent hardware/session acceptance checks. Do not convert unit-test results into Apple/Android interoperability claims.

## Primary references

- [Linux wireless iw/nl80211](https://wireless.docs.kernel.org/en/latest/en/users/documentation/iw.html)
- [NetworkManager device D-Bus properties](https://www.networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.Device.html)
- [Linux cfg80211 interface combinations](https://docs.kernel.org/driver-api/80211/cfg80211.html)
- [systemd execution sandbox](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html)
