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
