#!/usr/bin/env python3
"""Install/remove only LinuxDrop's Thunar action; preserve unrelated actions."""
import argparse
import fcntl
import os
from pathlib import Path
import stat
import tempfile
import xml.etree.ElementTree as ET

ACTION_ID = "1730000000000001-linuxdrop"


def update(config: Path, remove: bool = False) -> bool:
    config.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    lock = os.open(str(config) + ".linuxdrop.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        original = b""
        mode = 0o600
        try:
            fd = os.open(config, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        except FileNotFoundError:
            pass
        else:
            with os.fdopen(fd, "rb") as stream:
                info = os.fstat(stream.fileno())
                if not stat.S_ISREG(info.st_mode) or info.st_size > 2 * 1024 * 1024:
                    raise ValueError("Thunar configuration must be a regular XML file below 2 MiB")
                original = stream.read()
                mode = stat.S_IMODE(info.st_mode)
        if b"<!DOCTYPE" in original.upper() or b"<!ENTITY" in original.upper():
            raise ValueError("External or embedded XML entities are not supported")
        parser = ET.XMLParser(target=ET.TreeBuilder(insert_comments=True))
        root = ET.fromstring(original, parser=parser) if original else ET.Element("actions")
        if root.tag != "actions":
            raise ValueError("Unexpected Thunar XML root; configuration was not changed")
        actions = [a for a in root.findall("action") if a.findtext("unique-id") == ACTION_ID]
        if remove and not actions:
            return False
        for action in actions:
            root.remove(action)
        if not remove:
            action = ET.SubElement(root, "action")
            for key, value in [("icon", "io.github.marius4lui.LinuxDrop"), ("name", "Send with LinuxDrop"),
                               ("unique-id", ACTION_ID), ("command", "/usr/bin/linuxdrop send -- %F"),
                               ("description", "Share files with nearby devices"), ("patterns", "*")]:
                ET.SubElement(action, key).text = value
            for key in ("audio-files", "image-files", "other-files", "text-files", "video-files"):
                ET.SubElement(action, key)
        data = ET.tostring(root, encoding="utf-8", xml_declaration=True)
        if data == original:
            return False
        fd, name = tempfile.mkstemp(prefix=".linuxdrop-uca-", dir=config.parent)
        try:
            os.fchmod(fd, mode)
            with os.fdopen(fd, "wb") as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            # A private backup is created once without replacing previous recovery data.
            if original:
                try:
                    backup = os.open(str(config) + ".linuxdrop-backup", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
                except FileExistsError:
                    pass
                else:
                    with os.fdopen(backup, "wb") as stream:
                        stream.write(original)
            os.replace(name, config)
            directory = os.open(config.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            if os.path.exists(name):
                os.unlink(name)
        return True
    finally:
        os.close(lock)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--config", type=Path, default=Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))) / "Thunar/uca.xml")
    args = parser.parse_args()
    try:
        changed = update(args.config, args.remove)
    except (OSError, ValueError, ET.ParseError) as error:
        parser.exit(1, f"LinuxDrop: {error}\n")
    print("Thunar action updated. Reopen Thunar to load it." if changed else "Thunar action already up to date.")
