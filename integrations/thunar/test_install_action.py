import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-action.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallerTest(unittest.TestCase):
    def test_preserves_unrelated_actions_idempotent_remove_and_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "uca.xml"
            original = b'<actions><!-- Keep me --><action><name>Custom</name><command>custom %F</command></action></actions>'
            config.write_bytes(original)
            self.assertTrue(installer.update(config))
            self.assertFalse(installer.update(config))
            root = ET.parse(config).getroot()
            self.assertEqual(root.find("action/command").text, "custom %F")
            self.assertEqual(len(root.findall("action")), 2)
            self.assertEqual(Path(str(config) + ".linuxdrop-backup").read_bytes(), original)
            self.assertTrue(installer.update(config, remove=True))
            self.assertEqual(len(ET.parse(config).getroot().findall("action")), 1)
            self.assertFalse(installer.update(config, remove=True))
            link = Path(directory) / "link.xml"
            link.symlink_to(config)
            with self.assertRaises(OSError):
                installer.update(link)


if __name__ == "__main__":
    unittest.main()
