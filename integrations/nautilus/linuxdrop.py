"""Nautilus 43+ menu provider. File paths are argv entries, never shell text."""
from gi.repository import GObject, Nautilus, Gio


class LinuxDropMenu(GObject.GObject, Nautilus.MenuProvider):
    def get_file_items(self, files):
        paths = []
        for item in files:
            file = Gio.File.new_for_uri(item.get_uri())
            path = file.get_path()
            if item.is_directory() or path is None:
                return []
            paths.append(path)
        if not paths:
            return []
        menu = Nautilus.MenuItem(name="LinuxDrop::send", label="Send with LinuxDrop", tip="Share these files with a nearby device")
        menu.connect("activate", self._send, paths)
        return [menu]

    @staticmethod
    def _send(_item, paths):
        Gio.Subprocess.new(["/usr/bin/linuxdrop", "send", "--", *paths], Gio.SubprocessFlags.NONE)
