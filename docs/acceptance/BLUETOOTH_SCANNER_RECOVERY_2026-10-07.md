# Automatic Quick Share announcement-scanner recovery

The FastInit announcement listener now has its own supervisor. It starts even
when the configured controller is unavailable, retries after 2 seconds with
bounded exponential backoff to 15 seconds, and resets that delay after becoming
ready. Its read-only controller monitor checks the captured BlueZ owner and
selected controller's power state. A persistent monitoring future detects loss
even while the scan is intentionally paused for advertising or a connection.

Each failed generation finishes its acknowledged cleanup before another starts.
Unconfirmed cleanup is terminal and remains visible to backend shutdown. Cancelling
startup or backoff returns promptly; cancelling an active scan still waits for
cleanup. The worker does not restart LAN listeners or other transfer tasks.

Readiness is explicit: a confirmed scan or a monitored intentional pause updates
the component state, clearing only the listener's old error. Pauses have their
own explanation. Other Bluetooth-role errors are retained. An initially absent
explicit controller remains selected rather than falling back to another radio.

The BlueZ source confirms that power-off clears discovery clients, while a later
StopDiscovery returns NotReady. Cleanup accepts that only after confirming both
Powered=false and Discovering=false on the original daemon. A removed adapter is
accepted only when its absence is confirmed through that owner's ObjectManager.
Failed cleanup on a live controller with a remaining scan is still refused.
Repeated signal-rule removal no longer panics when the bus already removed it.

The private-bus fixture runs the actual scanner and supervisor. It covers initial
unavailability, protected radio airtime, power loss while paused and while scanning,
power restoration, controller removal/reappearance, daemon replacement without
controller fallback, terminal cleanup refusal and cancellation during retry.
The broader actor test synchronizes on this worker's reported state, not an
adapter-wide counter shared with outgoing recipient discovery.

Both private-bus suites passed; the final Quick Share actor scenario took 20.35
seconds. Four Quick Share library tests passed, including the consented UKEY2
exact-byte loopback. Network/Quick Share/daemon all-target Clippy and the modified
vendor libraries' Clippy passed with warnings denied. Package hashes and packaged
daemon D-Bus/HTTPS acceptance are recorded in
`dist/BUILD_REPORT_0.1.0+review.20261007.19.json`.

This is not full Bluetooth-role recovery: GATT, L2CAP, sender advertisements and
recipient discovery still need corresponding supervision. GATT/L2CAP currently
own both BLE bridge work and transferred Wi-Fi sessions; future recovery must
separate those lifetimes before restarting their listeners. Physical controller
and Apple/Android acceptance remains separate. The live demo was untouched.

Primary behavior reference: BlueZ's
[adapter implementation](https://github.com/bluez/bluez/blob/master/src/adapter.c),
particularly settings_changed, adapter_stop, remove_discovery_list and stop_discovery.
No upstream implementation code was copied.
