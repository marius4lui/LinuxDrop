# Shared Bluetooth advertising

AirDrop wake and Quick Share's sender/receiver now use one fair advertising queue
in linuxdrop-network. Previously AirDrop could retain the controller's only slot
indefinitely, leaving Quick Share unable to advertise. AirDrop now holds its
advertisement for three seconds, confirms removal and pauses for five seconds.
Quick Share's receiver yields between connections when a nudge is queued. GATT
and L2CAP services remain independent of advertisement rotation.

The scan-suppression guard is shared too: AirDrop windows pause Quick Share
scanning, and established Quick Share connections defer AirDrop wake. Admission
also checks this guard when no receiver holds a turn. Cancellation while queued
does not register an advertisement. Unconfirmed removal prevents replacement;
capacity checks still protect unrelated applications' advertisements.

The daemon has one Bluetooth-controller preference for all protocols. The queue
is process-wide and deliberately conservative even when automatic recovery picks
a different controller. Explicit selection is retained by both protocols' scan,
advertise, GATT and L2CAP paths; no new default-adapter fallback was introduced.

The private BlueZ regression runs both real advertising workers on one slot.
It verifies two successive AirDrop/Quick Share cycles, a protected connection,
one persistent GATT registration, selected-controller recovery and acknowledged
shutdown. The existing Quick Share sender/receiver lifecycle regression also
passes, preserving sender fairness, connection protection and owner recovery.

Physical advertising packets, RF timing and Android/Apple interoperability still
require actual devices. This record closes software coordination, not those
hardware acceptance requirements.
