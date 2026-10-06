# LinuxDrop – Architektur und Implementierungsreport

Stand: 6. Oktober 2026. Dieser Report konkretisiert die Projektidee; er dokumentiert noch keine implementierte oder getestete Produktfunktion.

Ergänzung: Der [vollständige Agentenplan](AGENT_IMPLEMENTATION_PLAN.md) konkretisiert die Notch, getrennte Bedienflächen, das Einstellungsinventar, Hardwareerkennung und ausführbare Arbeitspakete. Für diese Details ist der Agentenplan maßgeblich.

## 1. Verifizierter Ausgangspunkt

Repository: https://github.com/marius4lui/LinuxDrop. Beim Klonen enthält `main` am Commit `6a62038` nur README.md. Es gibt keine bestehende Anwendung, Build-Konfiguration oder Lizenzdatei. Es ist ein Neuaufbau, kein Umbau vorhandener Software.

Lokaler Checkout: `C:\Users\Marius\Projekte\Dev\LinuxDrop`. Auf dem Windows-Host sind Git und GitHub CLI verfügbar; `cargo` und `rustc` wurden im PATH nicht gefunden. WSL enthält Docker Desktop und Convertibled-Demo. Die bestehende Demo-Distribution wird nicht für LinuxDrop verändert. Für native Entwicklung wird eine eigene Linux-Umgebung benötigt; echte WLAN-Tests brauchen einen Linux-Host mit direktem Hardwarezugriff.

## 2. Produktentscheidung

Eine native Linux-App bündelt Geräte, Empfangsanfragen und Transfers. Rust übernimmt Core und Benutzerdienst; GTK4/libadwaita die Oberfläche. GNOME Shell und Dateimanager sind zusätzliche Clients desselben Dienstes. LocalSend bildet die erste vollständige vertikale Funktion. Quick Share folgt auf LAN, AirDrop zunächst als separat validierter experimenteller Pfad. Ein eigener AWDL-Neuaufbau ist kein Startziel.

Quick Share und Nearby Share erhalten ein gemeinsames Backend. Linux-zu-Linux benötigt zunächst kein viertes Protokoll: LocalSend deckt diesen Fall ab. Sichtbarkeit, Verfügbarkeit und Übertragung sind getrennte Zustände. Eine eingeschaltete Option darf bei fehlendem Backend niemals „Ready“ anzeigen.

## 3. Korrekturen und offene Forschungsfragen

- [LocalSend](https://github.com/localsend/protocol) dokumentiert v2.2, UDP-Multicast und HTTP(S), Vorbereitung, Upload, Abbruch und Reverse-Transfer. Startumfang ist die Upload-Strecke; „vollständiges v2.2“ darf erst nach Prüfung der weiteren API behauptet werden. Ein angekündigter Fingerprint ist kein Beweis für eine vertrauenswürdige Person.
- [RQuickShare](https://github.com/Martichou/rquickshare) beschreibt LAN als Voraussetzung und Bluetooth-Werbung als Hilfe für Android-mDNS. Daher BLE früh prototypisieren; sein tatsächlicher Bedarf entscheidet sich an Android-Gerätetests. Wi-Fi Direct bleibt ein eigener Forschungsmeilenstein.
- [opendrop-rs](https://github.com/ayourtch-llm/opendrop-rs) nennt echten iOS-18-Empfang, GPLv3 und fehlenden BLE-Wakeup zum ruhenden iPhone. Daraus folgt keine Zusage für LinuxDrop oder alle aktuellen iOS-Versionen. Erst isolierten Empfang reproduzieren, dann Integration entscheiden.
- [OWL](https://github.com/seemoo-lab/owl) verlangt aktiven Monitorbetrieb mit Injection und beschreibt exklusive AWDL-Nutzung der WLAN-Karte. Ein zweiter Adapter und geprüfte Wiederherstellung sind Voraussetzung für automatische Aktivierung. WSL-Builds ersetzen keine Funkabnahme.
- AR9271 bleibt Kandidat. USB-ID, Treiber, Band, regulatorisch zulässige Kanäle, Interface-Kombinationen und tatsächliche Datenframe-Injection müssen am Gerät geprüft werden. Monitorfähigkeit allein belegt keine AirDrop-Funktion.
- Packet, NearDrop und airdrop-mt7921 sind zusätzliche Referenzkandidaten. Ihre Eignung und Lizenzen sind in diesem Report noch nicht abschließend geprüft. Aussagen über iOS-26-Interop werden nicht als verifiziert übernommen.

## 4. Architektur und Repository

Zunächst nur tatsächlich benötigte Crates erstellen:

```text
Cargo.toml
crates/linuxdrop-core/       Geräte, Zustände, Fehler, Backend-Vertrag
crates/linuxdrop-daemon/     Lebenszyklus, Einstellungen, Transferkoordination
crates/linuxdrop-ipc/        versionierte D-Bus-Verträge und Client
crates/linuxdrop-localsend/  erstes Netzwerkbackend
app/linuxdrop/              GTK4/libadwaita, CLI-Einstieg
docs/                       Entscheidungen, Protokolle, Abnahme
tests/fixtures/             kleine, dokumentierte Protokollbeispiele
packaging/systemd/          Benutzerdienst
```

Hardware, Quick Share, AirDrop, netd und Shell-Erweiterung kommen mit ihren Meilensteinen hinzu. Leere Platzhalter für vermeintlich fertige Backends vermeiden. Core darf GTK, BlueZ und Root-Operationen nicht benötigen. Tokio ist der vorgeschlagene Netzwerkexecutor, zbus die D-Bus-Schicht; die GTK-Hauptschleife erhält Ereignisse über einen begrenzten Kanal.

Backend-Vertrag: Fähigkeiten und Status abfragen, starten/stoppen, Sichtbarkeit setzen, Transfer anbieten/senden, eingehende Anfrage beantworten und abbrechen. Discovery liefert fortlaufende Ereignisse statt nur eines einmaligen Gerätevektors. Jeder Aufruf besitzt Timeout, typisierte Fehler und einen Cancellation-Pfad. Die zentrale Transferkoordination vergibt IDs und verwaltet Zustände.

Geräteidentität beginnt als `(BackendId, BackendPeerId)`. Gleicher Name oder gleiche IP reichen nicht zum Zusammenführen. Eine globale Geräteansicht kann verifizierte Zuordnungen später ergänzen. Bluetooth ist ein separater Controller, keine Eigenschaft jeder WLAN-Karte. Hardwarefähigkeiten unterscheiden gemeldete, getestete und unbekannte Werte.

## 5. D-Bus und Transfermodell

Session-Bus-Name: `io.github.marius4lui.LinuxDrop`; versioniertes Interface: `io.github.marius4lui.LinuxDrop.Manager1`. Objektpfad: `/io/github/marius4lui/LinuxDrop`.

Startvertrag: ListPeers, ListTransfers, Send, AcceptTransfer, RejectTransfer, CancelTransfer, SetVisibility und GetBackendStates. Geräte-, Transfer- und Backend-Ereignisse enthalten stabile IDs und eine Revisionsnummer. Clients holen nach Dienstneustart einen Snapshot und setzen ihre Ansicht neu auf. Signal und Snapshot müssen ohne Ereignislücke synchronisierbar sein. Dateiliste und Strings erhalten feste Größenlimits.

Transferzustände: angeboten → wartet auf Entscheidung → angenommen → überträgt → verifiziert → abgeschlossen. Alternativen: abgelehnt, abgebrochen, fehlgeschlagen. Terminalzustände sind endgültig. Mehrere Dateien haben einzelne Zustände und Bytezähler; Teilfehler werden nicht als Gesamterfolg dargestellt. Ein abgebrochener Transfer kann nicht durch verspätete Netzwerkereignisse abgeschlossen werden.

Der Dienst öffnet die GTK-App über einen registrierten Anwendungseinstieg. D-Bus überträgt lokale, validierte Pfade oder Dateideskriptoren; Portal-/Sandbox-Verhalten wird vor Flatpak festgelegt. GNOME erhält keinen direkten Netzwerk- oder Hardwarezugriff.

## 6. Sichere Empfangspipeline

Metadaten prüfen → Zustimmung → reservierte temporäre Datei → begrenztes Streaming → Längenprüfung und verfügbare Protokollintegritätsprüfung → atomarer Abschluss → Ergebnis melden.

Dateinamen als einzelne Basenames behandeln; absolute Pfade, Separatoren, Traversal, NUL und überlange Namen ablehnen. Nicht nur `../` entfernen. Temporäre Dateien innerhalb des Ziel-Dateisystems exklusiv anlegen; Linux-Dateideskriptor-Operationen mit Schutz vor Symlinks verwenden. Zielkollisionen mit exklusivem No-Replace-Abschluss behandeln; gewöhnliches Rename kann Dateien überschreiben. Empfangsordner und Dateirechte vom Dienst kontrollieren.

Grenzen: Dateianzahl, Einzel-/Gesamtgröße, Metadatenlänge, parallele Transfers, offene Verbindungen und Zeit ohne Fortschritt. Freier Speicher ist eine Vorprüfung, kein Garant; Schreibfehler müssen sauber abbrechen. Archive später mit eigenen Extraktionslimits implementieren. Eine lokal berechnete Prüfsumme beweist ohne authentifizierten Vergleichswert keine Senderauthentizität.

Automatisch öffnen ist standardmäßig aus. Erst nach erfolgreichem Abschluss und per Desktop-API öffnen; niemals Netzwerkdateinamen als Shell-Code verwenden. Tokens und sensible Pfade nicht protokollieren. Nach Neustart verwaiste temporäre Dateien kontrolliert bereinigen.

## 7. Hardware und Privilegien

Hardwaremanager liest udev, nl80211 und NetworkManager; BlueZ ergänzt Bluetooth. Auswahl berücksichtigt aktuelle Verbindung, Interface-Kombinationen und vorherige Tests. Ein aktiver Internetadapter wird nicht automatisch in Monitorbetrieb versetzt. Ein dedizierter geprüfter Adapter wird bevorzugt.

netd entsteht erst mit dem AWDL-Prototyp. GUI und Transferdienst laufen ohne Root. Der Helper erhält nur benötigte Capabilities, einen eng begrenzten API-Vertrag und autorisierte lokale Clients. Keine frei übergebenen Befehle, Shellfragmente oder beliebigen Interface-Namen. Verbindungsstatus, Kanal und NetworkManager-Zuständigkeit vor Änderungen sichern; Stop, Fehler, Hot-Unplug und Neustart müssen Wiederherstellung auslösen. Ein Capability-Set allein ist keine Zugriffskontrolle. systemd-Härtung muss die benötigten Raw-Socket-/TAP-Operationen weiterhin zulassen und separat getestet werden.

## 8. Oberfläche

libadwaita bildet Typografie, Abstände, Navigation, Tastaturbedienung und Hell-/Dunkelmodus. Zurückhaltende eigene Gestaltung statt einer zweiten Designsprache über GTK. Kleine Fenster erhalten eine adaptive Navigation, große Fenster eine Seitenleiste.

**In der Nähe:** Sichtbarkeit mit kurzer Erklärung, Geräte als gut lesbare Liste, Protokollbadge und konkreter Verfügbarkeitsstatus. Datei-Dropzone und „Dateien auswählen“; Auswahl mehrerer Dateien mit anschließendem Empfänger. Leere Ansicht unterscheidet „Suche läuft“, „Keine Geräte“, „Netzwerk fehlt“ und „Backend nicht verfügbar“.

**Transfers:** Gerät, Richtung, Dateiname, Bytefortschritt, Abbrechen und konkrete Fehler. Empfangsanfrage zeigt Absender, Anzahl und Gesamtgröße; Annehmen/Ablehnen auch als Benachrichtigungsaktionen. Abschluss bietet Öffnen und Ordner anzeigen.

**Drahtlos:** Adapter und Bluetooth getrennt, aktive Verbindung, geprüfte Fähigkeiten und Ursache einer Einschränkung. „Experimentell“ bleibt bei AirDrop sichtbar. Hardwareaktionen erfolgen bewusst und mit Wiederherstellungsstatus.

**Einstellungen:** Name, Empfangsordner, Autostart, Benachrichtigungen, Sichtbarkeit und einzelne Backends. Verfügbarkeit erklärt deaktivierte Optionen. Sichtbarkeit startet konservativ; zeitlich begrenzte öffentliche Sichtbarkeit wird vor Release festgelegt. „Kontakte“ nur anbieten, wenn das Backend echte Identitätsprüfung unterstützt.

GNOME-Integration folgt nach stabiler D-Bus-API. Quick Settings zeigt Status und Aktionen; die Transfer-Pill ist optional, tastaturbedienbar und respektiert reduzierte Bewegung. Keine zweite Annahmeentscheidung neben der Haupt-App. GNOME-Versionen anhand tatsächlich getesteter Shell-Versionen freigeben.

## 9. Umsetzung mit Abnahmekriterien

| Schritt | Ergebnis | Abnahme |
|---|---|---|
| M0 | Rust-Workspace, Zustandsmodell, D-Bus, Linux-CI | fmt/clippy/tests; Dienstneustart und IPC-Vertrag geprüft |
| M1 | LocalSend Discovery und Empfang | offizieller Client, Zustimmung/Ablehnen, mehrere Dateien, identische Bytes |
| M2 | LocalSend Senden und GTK-App | beide Richtungen, Fortschritt, Abbruch, Kollisionen, Schreibfehler; Desktop-Start ohne Terminal |
| M3 | Quick Share LAN mit erforderlichem BLE | echte Android-Interop, Vergleichscode, Ablehnen, Abbruch und verschlüsselter Transfer |
| M4 | Hardwareinventar und Auswahl | USB hinzufügen/entfernen; bestehende Internetverbindung bleibt nutzbar |
| M5 | isolierter AirDrop-Empfang | konkrete Apple-/Adapter-/Kernel-Matrix; keine automatische Netzübernahme |
| M6 | AirDrop-Senden und BLE-Wakeup | ruhendes und aktiv sichtbares iPhone separat testen; vollständige Dateien |
| M7 | GNOME und Nautilus | mehrere Dateien, Benachrichtigungsaktionen, Dienst-/Shell-Neustart |
| M8 | DEB, danach RPM/Arch | saubere VM-Installation, Upgrade, Deinstallation, Dienstrechte |
| später | Wi-Fi Direct, weitere Dateimanager, Flatpak | jeweils eigener nachgewiesener Nutzungsfall und Abnahme |

Kein Kalenderdatum für AirDrop oder 1.0 vor dem Hardware-Prototyp. Das wichtigste nächste Coding-Paket ist M0 plus ein kleiner LocalSend-Empfangspfad, nicht drei gleichzeitig begonnene Protokolle.

## 10. Tests, CI und Lizenz

Linux-CI trennt Core-/Protokolltests von GTK-Builds. Prüfen: Format, Clippy, sinnvolle Tests, später Dependency-Audit und Lizenzregeln mit festgelegten Tools. Schwerpunktfälle: verspätetes Accept nach Cancel, doppelte Events, Byte-/Größenabweichung, Traversal, Symlink, Kollision und Dienstneustart. Fuzzing gilt Netzwerkparsern und später Archiveingängen.

Interop-Protokoll enthält Betriebssystem, Clientversion, Hardware, Richtung, Dateitypen, Größen und Ergebnis. Ein synthetischer Test ersetzt weder einen offiziellen LocalSend-Client noch Android oder Apple. AWDL-Abnahme braucht Hardware, Paketmitschnitt und wiederholbare Recovery-Tests.

Das LinuxDrop-Repository hat noch keine Lizenz. GPLv3 ist eine sinnvolle vorgeschlagene Entscheidung bei geplantem Reuse, aber noch nicht festgelegt. Vor Fremdcodeübernahme exakte Commitstände, LICENSE-/SPDX-Angaben, Abhängigkeiten und Attribution inventarisieren. Bis dahin keine Referenzquellen hineinkopieren. Eine Prozessgrenze allein garantiert keine rechtliche Trennung.

## 11. Übergabe in Codex

Dieser Report ist der Einstiegspunkt für die nächste Implementierungsrunde. Der Checkout liegt außerhalb des ursprünglichen Chat-Arbeitsordners; Folgearbeit muss ausdrücklich diesen Repositorypfad verwenden. Die verfügbaren Codex-App-Werkzeuge haben bei dieser Prüfung keine Funktion zum Registrieren eines neuen lokalen Projekts bereitgestellt. Daher ist eine Projektregistrierung noch offen; das Öffnen dieses Reports im aktuellen Task ist davon getrennt.

Für das Telefon beschreibt [OpenAI Remote connections](https://learn.chatgpt.com/docs/remote-connections) die Verbindung mit einem laufenden Windows-/Mac-Host und Fortsetzung bestehender Tasks. Gerätepaarung und Sichtbarkeit dieses Tasks wurden hier nicht getestet. Falls bereits verbunden, diesen Task fortsetzen; andernfalls mobile Einrichtung in Codex abschließen und den Checkout als Projekt hinzufügen.

Nächster Auftrag: „Arbeite in C:\Users\Marius\Projekte\Dev\LinuxDrop. Lies docs/IMPLEMENTATION_REPORT.md und implementiere M0 und danach LocalSend-Empfang. Richte eine separate Linux-Entwicklungsumgebung ein; bestehende Demo-Umgebungen und aktive Netzverbindungen nicht verändern. Weise echte Interop-Abnahme getrennt von Build-/Unit-Tests aus.“
