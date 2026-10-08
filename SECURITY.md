# Security

LinuxDrop 0.1.0 is experimental software that accepts local-network input. Report suspected vulnerabilities privately to the repository maintainer through GitHub private vulnerability reporting if available; otherwise contact the maintainer before publishing exploit details. Do not attach personal files, credentials or unrelated network captures to public issues.

## Runtime boundaries

- The app and transfer daemon run as the logged-in user. The network helper runs as a separate system account with network capabilities; its bounded operation protocol checks peer credentials, the local session and polkit. It exposes no shell-command service.
- Incoming files require explicit approval. Quick Share additionally requires comparing the connection code at both ends. Names, addresses and discovery advertisements do not establish a contact's identity.
- AirDrop supports experimental Everyone mode, without contacts-only authentication. LocalSend's discovery fingerprint pins the selected transport certificate but does not authenticate an account.
- Files use private temporary storage, size/limit checks and publication without replacing existing names. Archives reject traversal and non-regular entries. Partial transfers are not completed files.
- Visibility starts hidden and can expire. Session lock blocks approval; hiding on lock is enabled by default. Active Internet adapters remain protected.
- Optional reverse download uses HTTP and an expiring PIN. Its confirmation identifies the lack of encryption. Use encrypted device transfers for confidential files.
- Opening received files automatically is disabled by default. Enabling it invokes the desktop file handler as your user and exposes that handler to untrusted documents.

## Verification limits

Tests exercise consent, certificate mismatch, hostile names, bounded archives, exact bytes, replay/cancellation and collisions. They do not certify every parser. No independent security audit or exhaustive fuzz campaign has been completed. Physical AWDL/BLE/Wi-Fi behavior and recovery remain part of device acceptance.

Keep protocol dependencies pinned, preserve vendor patch notes, and rerun relevant negative-path tests when modifying framing, cryptography, archives, privilege boundaries or file publication.
