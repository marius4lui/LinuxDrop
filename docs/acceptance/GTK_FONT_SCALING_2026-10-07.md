# GTK desktop text scaling acceptance

The application now uses relative font sizes for custom labels, including the
compact drop surface, protocol/status text and verification codes. GTK resolves
`em` against inherited font size, so these labels follow desktop font settings
and text scaling alongside native controls. The implementation remains compatible
with GTK 4.14; it does not depend on newer CSS custom properties.

Primary reference: [GTK CSS properties](https://docs.gtk.org/gtk4/css-properties.html).

The first native 150% render exposed horizontal clipping: the send action row
and drop zone forced 514 pixels into a 470-pixel content viewport. A scale-aware
`max-width: 440sp` breakpoint now stacks these controls. The drop icon/title stay
together, and their entire heading is hidden once files are selected. Native
bottom navigation now fits because the page no longer forces its parent wider.

## Verification

Ubuntu 24.04, GTK 4.14/libadwaita 1.5, isolated Xvfb/private session bus,
German UI, 480x600 requested window, desktop font DPI 96 to 144:

- Existing native regression scenario passes (18.40 seconds), including draft,
  focus, protocol selection, settings failure/retry and incoming consent paths.
- Real custom-label Pango metrics increase with desktop DPI; a regression to
  fixed pixel text fails the height-ratio assertion.
- Root minimum content width stays within the compact viewport at large text.
- Main send, settings, hardware, drop surface and incoming verification renders
  were inspected. The drop surface fits within 480x500 at large text.
- App all-target Clippy with warnings denied passes.

Captures: [send](ui/gtk-font-scaling/send-compact-large-text.png),
[settings](ui/gtk-font-scaling/settings-compact-large-text.png),
[hardware](ui/gtk-font-scaling/hardware-compact-large-text.png),
[drop surface](ui/gtk-font-scaling/drop-surface-large-text.png),
[incoming review](ui/gtk-font-scaling/incoming-compact-large-text.png).

Large text intentionally uses scrolling for page content; native tab labels can
ellipsize while their icons and accessible labels remain. This is not a claim
of physical screen-reader speech, multi-monitor/fractional-scaling acceptance,
or external file-drag delivery. The live demo was not restarted or replaced.
Fedora revision 3 / Arch revision 2 packages predate this source change and
must be rebuilt before claiming they include it.

## Release binary and Ubuntu package follow-up

Ubuntu DEB `0.1.0+review.20261007.12` was built from source base `a0a5199`.
The release workspace and AirDrop helper builds, all four executable library
checks, desktop/schema validation, and packaged daemon private-network
D-Bus/HTTPS consent/transfer scenario pass.

- DEB SHA256: `4891858c61a59e86315ddf7d25acd22c026a8d187be0a48c42c617c896720fe4`
- Matching source SHA256: `3f48fdbb24ee68d754f4948cd5cdc33727cda086f05b3cb81ca7a89c7f21d011`
- Detailed package report: `dist/BUILD_REPORT_0.1.0+review.20261007.12.json`

The current release executable was selected through PATH for an isolated
GNOME 46 Wayland smoke (German, 800x600, 150% desktop text). Actual drop-window
launch, measured centering, spacing changes, close/reopen, existing Shell
interactions and preferences synchronization all pass. The resulting
[Wayland capture](ui/gtk-font-scaling/drop-shell-46.png) was visually inspected.
No installation or restart in the live desktop was performed.
