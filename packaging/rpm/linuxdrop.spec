Name:           linuxdrop
Version:        0.1.0
Release:        4%{?dist}
Summary:        Nearby file sharing for Linux
License:        GPL-3.0-only
URL:            https://github.com/marius4lui/LinuxDrop
Source0:        linuxdrop_%{version}_source.tar.gz
%bcond_with prebuilt
%if %{with prebuilt}
Source1:        linuxdrop-binaries.tar.gz
BuildRequires:  glib2 systemd-rpm-macros
%else
BuildRequires:  cargo rust gcc gcc-c++ cmake pkgconfig(gtk4) pkgconfig(libadwaita-1) pkgconfig(openssl) pkgconfig(dbus-1) pkgconfig(libudev) pkgconfig(libnl-3.0) pkgconfig(libnl-genl-3.0) libpcap-devel libev-devel protobuf-compiler glib2-devel
BuildRequires:  systemd-rpm-macros
%endif
Requires:       gtk4 >= 4.12
Requires:       libadwaita >= 1.5
Requires:       openssl-libs systemd dbus polkit iw iproute ethtool python3 busybox
Recommends:     NetworkManager bluez nautilus-python
%{?systemd_requires}

%description
Native desktop nearby file sharing. Experimental AirDrop requires a dedicated wireless adapter.

%prep
%setup -q -c -n LinuxDrop-%{version}
%if %{with prebuilt}
mkdir -p target/release
tar -xf %{SOURCE1} -C target/release
%endif

%build
%if !%{with prebuilt}
# cc-rs caches distro CFLAGS before the jitterentropy -O0 guard in aws-lc-sys
# 0.45.0. The upstream CMake builder applies its per-source flags last.
export AWS_LC_SYS_CMAKE_BUILDER=1
cargo build --release --locked --workspace
cargo build --release --locked --manifest-path vendor/opendrop-rs/Cargo.toml -p filin-rs
%endif

%install
%if %{with prebuilt}
LINUXDROP_FILIN="$PWD/target/release/filin" DESTDIR=%{buildroot} sh packaging/install.sh
%else
DESTDIR=%{buildroot} sh packaging/install.sh
%endif
install -Dm644 packaging/linuxdrop.sysusers %{buildroot}%{_sysusersdir}/linuxdrop.conf

%pre
%sysusers_create_compat packaging/linuxdrop.sysusers

%post
%systemd_post linuxdrop-netd.service

%preun
%systemd_preun linuxdrop-netd.service

%postun
%systemd_postun_with_restart linuxdrop-netd.service

%files
%{_bindir}/linuxdrop
%{_bindir}/linuxdropd
%{_bindir}/linuxdrop-thunar-install
%{_libexecdir}/linuxdrop/
/usr/lib/systemd/user/linuxdropd.service
/usr/lib/systemd/system/linuxdrop-netd.service
%{_sysusersdir}/linuxdrop.conf
%{_datadir}/applications/io.github.marius4lui.LinuxDrop.desktop
%{_datadir}/metainfo/io.github.marius4lui.LinuxDrop.metainfo.xml
%{_datadir}/dbus-1/services/io.github.marius4lui.LinuxDrop.service
%{_datadir}/dbus-1/system.d/io.github.marius4lui.LinuxDrop.Netd.conf
%{_datadir}/polkit-1/actions/io.github.marius4lui.LinuxDrop.policy
%{_datadir}/polkit-1/rules.d/50-linuxdrop-netd.rules
%{_datadir}/nautilus-python/extensions/linuxdrop.py
%{_datadir}/kio/servicemenus/linuxdrop.desktop
%{_datadir}/Thunar/sendto/linuxdrop.desktop
%{_datadir}/gnome-shell/extensions/linuxdrop@marius4lui.github.io/
%{_datadir}/icons/hicolor/scalable/apps/io.github.marius4lui.LinuxDrop.svg
%{_datadir}/doc/linuxdrop/
