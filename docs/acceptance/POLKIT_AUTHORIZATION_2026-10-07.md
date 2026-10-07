# Real Polkit authorization and packaging correction - 2026-10-07

The booted Ubuntu acceptance environment exposed a real product failure missed
by process/namespace fixtures: linuxdrop-netd runs as its dedicated system user,
but its Polkit action did not name that account as an action owner. Consequently
pkcheck was forbidden to query authorization for the desktop client's different
identity, before any administrator challenge could occur.

The policy now annotates only the manage-radio action with
`org.freedesktop.policykit.owner=unix-user:linuxdrop-netd`. This is the mechanism
ownership facility documented by the
[official Polkit manual](https://polkit.pages.freedesktop.org/polkit/polkit.8.html).
The action still denies nonlocal/inactive subjects and requires
`auth_admin_keep` for an active local subject. The annotation permits the
mechanism's authorization query; it does not grant the requesting user the action.

Two reusable probes exercise the installed non-root helper, real systemd/logind
PAM sessions, pkttyagent, the stock administrator authentication stack and Polkit.
They require the disposable-system marker and root opt-in. A synthetic test
password is generated in memory, passed to chpasswd via stdin and to the terminal
agent via a private PTY, never logged or put in arguments; the test account is
locked in finally. Existing desktop sessions and the LinuxDrop-Dev demo are not
changed. The root-created test sessions use the current real virtual-console
seat/VT and a separate inactive VT; no virtual-terminal switch is performed.

Observed acceptance after the policy correction:

- Inactive seat session: rejected by the helper's local-session guard.
- Seatless PAM session: denied by the unchanged Polkit authorization policy.
- Active local session without agent: denied.
- Wrong administrator password: a real prompt appears and authentication fails.
- Correct administrator password: a real prompt appears and the request passes
  through authorization to radio selection, which returns `radio not found` for
  the intentionally nonexistent test adapter. No actual radio is mutated.

The baseline without the annotation returned Polkit's trusted-caller error.
After adding it, a seatless session correctly remained denied; a real seat-bound
session produced the expected password challenge. This distinction is preserved
in the assertions, not bypassed by a permissive test rule.

The extended matrix also passes against installed package `.35` on booted
Ubuntu, without changing its policy or binaries:

- The actual `linuxdropd.service` runs in the user's systemd manager, outside
  the login session cgroup. A D-Bus `RunHardwareDiagnostic` call triggers real
  administrator authentication for that daemon PID and reaches radio selection.
  All transport backends are disabled for this test account; its settings are
  restored and the service stopped afterwards. The PAM session carries real
  logind graphical-session metadata; this does not test a rendered GUI agent.
- A root-created `/bin/login -h` PAM session is verified as remote by logind
  and denied without a password prompt.
- With an independently verified active local session for the same UID held
  open, the remote caller is still denied by Polkit without a prompt. It cannot
  borrow the local session's authority.

The first remote CI run (`37646105711`) passed build, Clippy, tests, fuzzing,
dependency policy and the installed systemd probe, but failed to obtain a result
from the first Polkit client. The test now copies its non-secret client to a
root-owned readable temporary directory, removing dependency on runner-home
permissions, and reports safe unit/session state on failure. Raw terminal buffers
remain suppressed. Package CI installs required dependencies without optional
desktop recommendations, avoiding an unrelated display-manager installation
during this test. A fresh remote CI result is still required; the initial failure
is not counted as a pass.

Graphical authentication-agent behavior and physical adapter mutation remain
separate acceptance work. These probes use real services and authorization but
an intentionally nonexistent radio, so they do not establish hardware support.
