# WSLg nested GNOME rendering diagnosis

2026-10-06: GNOME 46's nested desktop could render an internal screenshot while
the actual Windows-visible outer window stayed black. Software rendering alone
(`LIBGL_ALWAYS_SOFTWARE=1`, `GALLIUM_DRIVER=llvmpipe`) did not resolve it.

The compositor reported `MESA: error: Failed to attach to x11 shm`. A minimal
non-window XShm test against WSLg display `:0` isolated the cause:

| WSL user | Result |
| --- | --- |
| `linuxdrop`, UID 1001 | XShm attach rejected: X11 error 10 (BadAccess), major 130, minor 1 |
| original `ubuntu`, UID 1000 | XShm attach succeeded, no X11 errors |

The same 4096-byte private memory segment, mode 0600, was used in each test and
removed afterward. Reproduction: `python3 app/linuxdrop/tests/probe-xshm.py`
under each user. This test opens no window and changes no existing shared-memory
permissions.

WSLg's Xwayland process uses UID 1000. Mesa's EGL X11 software presentation
therefore cannot share the UID 1001 client's private memory with that server.
GNOME 46's nested Wayland backend explicitly selects EGL Xlib and copies its
offscreen views to an X11 onscreen framebuffer. This is why internal scene
rendering can succeed without successful outer presentation.

Use the original UID 1000 user for this dedicated WSL nested-desktop demo. Do
not weaken memory permissions, run the GUI as root, or modify WSLg globally.
The ordinary GTK app using native WSLg Wayland and Cairo can run under UID 1001;
that separate live app was visibly verified by the parent agent. A native Linux
desktop normally runs its display server and GUI clients under the same user.

The working GNOME 50 development demo uses a different path: `--devkit` streams
the compositor through PipeWire to a GTK viewer. Its successful rendering did
not validate GNOME 46's older EGL X11 presentation path.

Primary implementation references:

- [GNOME 46 X11 renderer selects EGL for Wayland](https://gitlab.gnome.org/GNOME/mutter/-/blob/gnome-46/src/backends/x11/meta-renderer-x11.c)
- [GNOME 46 nested framebuffer presentation](https://gitlab.gnome.org/GNOME/mutter/-/blob/gnome-46/src/backends/x11/nested/meta-stage-x11-nested.c)

The UID 1000 launch still requires checking the actual Windows-visible desktop
and input. A successful XShm probe is causal evidence for the fix, not a
substitute for that final presentation check.
