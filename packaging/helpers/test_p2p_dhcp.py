"""Exercise DHCP input validation without touching interfaces or root paths."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("p2p_dhcp", Path(__file__).with_name("p2p-dhcp.py"))
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)


class DhcpValidation(unittest.TestCase):
    def setUp(self):
        self.values = {"interface": "p2p-wlan2-0", "LINUXDROP_P2P_INTERFACE": "p2p-wlan2-0",
                       "LINUXDROP_LEASE_ID": "a" * 32, "ip": "192.168.49.2", "subnet": "255.255.255.0"}

    def test_accepts_address_but_does_not_interpret_other_options(self):
        self.values.update(router="10.0.0.1", dns="1.1.1.1", hostname="$(command)", staticroutes="untrusted")
        self.assertEqual(hook.validated_lease(self.values), ("p2p-wlan2-0", "a" * 32, "192.168.49.2", 24))

    def test_rejects_foreign_interfaces_and_unsafe_addresses(self):
        for key, value in [("interface", "wlan0"), ("interface", "../../etc"), ("LINUXDROP_LEASE_ID", "../bad"),
                           ("ip", "127.0.0.1"), ("ip", "0.0.0.0"), ("ip", "224.0.0.1"),
                           ("ip", "192.168.49.255"), ("ip", "192.168.49.0"), ("subnet", "0.0.0.0"),
                           ("subnet", "255.0.255.0")]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                hook.validated_lease({**self.values, key: value})


if __name__ == "__main__":
    unittest.main()
