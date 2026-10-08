"""Initialize the explicitly requested private logind fixture for Shell CI."""
import os
import pwd
import time
import dbus

assert os.environ["DBUS_SYSTEM_BUS_ADDRESS"] == os.environ["DBUS_SESSION_BUS_ADDRESS"]
assert os.environ.get("LINUXDROP_SMOKE_MOCK_LOGIND") == "1"
bus = dbus.SessionBus()
deadline = time.monotonic() + 5
while not bus.name_has_owner("org.freedesktop.login1"):
    if time.monotonic() >= deadline:
        raise RuntimeError("Private logind fixture did not start")
    time.sleep(0.02)
mock = dbus.Interface(bus.get_object("org.freedesktop.login1", "/org/freedesktop/login1"), "org.freedesktop.DBus.Mock")
session = mock.AddSession("linuxdrop_ci", "seat0", dbus.UInt32(os.getuid()), pwd.getpwuid(os.getuid()).pw_name, True)
mock.AddMethod("org.freedesktop.login1.Manager", "GetSessionByPID", "u", "o", "ret = " + repr(str(session)))
mock.AddMethod("org.freedesktop.login1.Manager", "GetUser", "u", "o", "ret = '/org/freedesktop/login1/user/' + str(args[0])")
