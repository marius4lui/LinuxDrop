import GLib from 'gi://GLib';
const german = GLib.get_language_names().some(language => language.startsWith('de'));
const de = {
    'Connecting…': 'Verbinden …', 'Nearby sharing': 'Teilen in der Nähe',
    'Open LinuxDrop': 'LinuxDrop öffnen', 'Close LinuxDrop': 'LinuxDrop schließen', 'Transfer in progress': 'Übertragung läuft',
    'Send files…': 'Dateien senden …', 'LinuxDrop settings': 'LinuxDrop-Einstellungen',
    'Notch preferences': 'Notch-Einstellungen', 'LinuxDrop sharing controls': 'LinuxDrop-Freigabe',
    'Offline': 'Offline', 'Service unavailable': 'Dienst nicht verfügbar', 'Needs attention': 'Problem aufgetreten',
    'Compare code': 'Code vergleichen', 'Request': 'Anfrage', 'Waiting': 'Warten',
    'Visible to everyone': 'Für alle sichtbar', 'Hidden': 'Unsichtbar', 'No devices nearby': 'Keine Geräte in der Nähe',
    'Compare this code on both devices': 'Vergleiche diesen Code auf beiden Geräten',
    'Decline': 'Ablehnen', 'Codes match': 'Codes stimmen überein', 'Accept': 'Annehmen', 'Cancel': 'Abbrechen', 'Details': 'Details',
    'Send files': 'Dateien senden', 'Drop files': 'Dateien ablegen', 'Settings': 'Einstellungen',
    'Open the drop area, add your files, and choose a nearby device.': 'Öffne die Ablagefläche, füge Dateien hinzu und wähle ein Gerät in der Nähe.',
    'Desktop notch': 'Desktop-Notch', 'Sharing controls': 'Freigabe-Steuerung',
    'Sharing controls that stay out of your way.': 'Schneller Zugriff auf Dateien und Übertragungen.',
    'Show panel button': 'Knopf in der oberen Leiste anzeigen', 'Click the LinuxDrop icon to open the bubble': 'Klicke auf das LinuxDrop-Symbol, um die Bubble zu öffnen',
    'Hide in full screen': 'Bei Vollbild verbergen', 'Keep videos and presentations unobstructed': 'Videos und Präsentationen nicht verdecken',
    'Animate transitions': 'Übergänge animieren', 'Also respects the desktop animation setting': 'Berücksichtigt auch die Animationseinstellungen des Desktops',
    'Position and behavior': 'Position und Verhalten', 'Monitor': 'Bildschirm',
    '−1 follows the primary monitor; otherwise use a monitor index': '−1 verwendet den Hauptbildschirm; sonst den Bildschirmindex angeben',
    'Top spacing': 'Abstand nach oben', 'Logical pixels below the panel': 'Logische Pixel unter der oberen Leiste',
    'Close after inactivity': 'Nach Inaktivität schließen', 'Seconds; 0 keeps the expanded menu open': 'Sekunden; 0 lässt das aufgeklappte Menü offen',
    'Files and privacy': 'Dateien und Privatsphäre',
    'Click the LinuxDrop icon in the top panel first. Then choose Drop files or drag files over the open bubble. Click outside or press Escape to close. Requests and progress never open it automatically.': 'Klicke zuerst auf das LinuxDrop-Symbol in der oberen Leiste. Wähle dann Dateien ablegen oder ziehe Dateien über die geöffnete Bubble. Ein Klick außerhalb oder Escape schließt sie. Anfragen und Fortschritt öffnen sie niemals automatisch.',
};
export function t(text) { return german ? (de[text] ?? text) : text; }
export function nearby(count) { return german ? `${count} Geräte` : `${count} nearby`; }
export function filesStatus(count, active) { return german ? `${count} Dateien · ${active} aktive Übertragungen` : `${count} files · ${active} active transfers`; }
