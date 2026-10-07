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

The probes are added to the Linux CI installed-package phase with python3-pexpect;
remote execution is pending. Graphical authentication-agent behavior, actual
user-service invocation, remote-session combinations and physical adapter
mutation remain separate acceptance work.
