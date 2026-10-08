# Long transfer filenames and cross-page width

The clipped Fedora file-manager capture was reproduced with the current app.
The cause was not Fedora-specific: an unbroken filename on the inactive
Transfers page forced the homogeneous ViewStack wider than its visible window.
A focused native regression failed before the fix: the content requested 4511
pixels inside a 470-pixel viewport. Earlier compact tests used short or hyphenated
filenames, so they did not exercise this input.

The shared GTK label helper now permits character fallback when a word cannot
fit. Transfer filename summaries are limited to two ellipsized lines with the
full text retained in their tooltip and underlying label. Peer names and the
filename summary use literal text rather than UI translation lookup.

## Verification

The existing native scenario now includes a long filename without whitespace or
hyphens. It asserts inactive-page minimum width, the two-line summary bound and
complete tooltip text, alongside the existing draft/settings/consent checks.

- Ubuntu 24.04 GTK 4.14/libadwaita 1.5: native scenario passed, 18.12 seconds.
- Fedora 44 GTK 4.22/libadwaita 1.9: same current test executable against Fedora
  runtime libraries passed, 18.25 seconds. This cross-runtime run is not yet a
  claim of a new native Fedora RPM build.
- App all-target Clippy with warnings denied passed.
- [Ubuntu transfer capture](ui/gtk-transfer-width/ubuntu-transfers-long-filename.png)
  was inspected; [Fedora capture](ui/gtk-transfer-width/fedora-transfers-long-filename.png)
  records the corresponding runtime check.

The installed file-manager integration fixture additionally checks actual
horizontal bounds for Add more files, Send files and Share with a link. This
closes the gap where exact selected filenames were correct but actions were
outside the window. Native package rebuild and installed-program checks follow.

## Native installed-package completion

Fedora revision 4 and Arch revision 3 were built natively from source base
`28524c0`, using their existing build caches. Fedora upgraded 3 -> 4, passed
`rpm -V`, retained user preferences, and ran all three installed file-manager
menu journeys with the new button-bound checks. All passed. The actual
[installed Fedora capture](ui/gtk-transfer-width/fedora-r4-nautilus-draft.png)
was inspected: header, footer, remove actions and navigation fit the window.
[Thunar](ui/gtk-transfer-width/fedora-r4-thunar-draft.png) and
[Dolphin](ui/gtk-transfer-width/fedora-r4-dolphin-draft.png) record the other paths.

Arch upgraded 2 -> 3, passed package integrity and preserved preferences. Its
installed GTK desktop-file selection, settings/hardware navigation, single
instance and retained draft check passed; removal removed the new Thunar
resource and retained preferences. These private roots are not booted desktops.

Ubuntu review `.14` passed release build, binary library resolution, resources,
source/payload comparisons and its packaged daemon private-network D-Bus/HTTPS
scenario. The user demo was not installed into or restarted.

Artifacts:

| Package | SHA256 |
| --- | --- |
| Ubuntu `0.1.0+review.20261007.14` | `0d46b5d428f18f55428652f79f8d88df4b0ad49cd69b080d0282652b3c05192d` |
| Fedora `0.1.0-4.fc44` | `7134c1107a8f44657352c9d27fcd64723a8abbd0353f182eb922d75fecc1ef88` |
| Arch `0.1.0-3` | `963b2ede546e56ed7571909e9a1656bf78da450abc843b27bf34c6e99214d69d` |

All use matching source SHA256
`446bcc98c31897a2b2c2c2698b28482a23219c0ce3aaaa21565ac95e2515f3df`.
Artifacts, exact logs, JSON build reports and checksum manifests are in `dist/`,
`dist/fedora-44/` and `dist/arch/`. The earlier Fedora clipping item is now closed;
full screen-reader, remaining desktop versions and booted-service acceptance are
still separate requirements.
