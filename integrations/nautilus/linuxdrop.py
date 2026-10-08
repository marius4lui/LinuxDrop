"""Nautilus 43+ menu provider. File paths are argv entries, never shell text."""
from gi.repository import GObject, Nautilus, Gio, GLib


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
        preferred = (language.split("_")[0].split(".")[0].split("-")[0]
                     for language in GLib.get_language_names())
        german = next((language for language in preferred if language in ("de", "en", "C")), "en") == "de"
        menu = Nautilus.MenuItem(
            name="LinuxDrop::send",
            label="Mit LinuxDrop senden" if german else "Send with LinuxDrop",
            tip="Diese Dateien mit einem Gerät in der Nähe teilen" if german else "Share these files with a nearby device",
            icon="io.github.marius4lui.LinuxDrop",
        )
        menu.connect("activate", self._send, paths)
        return [menu]

    @staticmethod
    def _send(_item, paths):
        Gio.Subprocess.new(["/usr/bin/linuxdrop", "send", "--", *paths], Gio.SubprocessFlags.NONE)
