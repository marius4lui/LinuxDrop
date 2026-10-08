#!/usr/bin/env python3
"""Real LocalSend wire request for native consent UI review (loopback only).

The test trusts the local test daemon's public certificate. It does not emulate
an Android/iOS client and is never a substitute for physical interoperability.
"""
import hashlib
import json
import pathlib
import ssl
import urllib.parse
import urllib.request

cert = pathlib.Path.home() / '.local/share/linuxdrop/localsend-cert.pem'
context = ssl.create_default_context(cafile=str(cert))
context.check_hostname = False  # local daemon identity is pinned by this public cert
base = 'https://127.0.0.1:53317/api/localsend/v2'
payload = b'LinuxDrop native GTK consent and byte verification\n'
request = {
    'info': {'alias': 'LocalSend UI test client', 'version': '2.1', 'deviceModel': 'Loopback wire fixture',
             'deviceType': 'desktop', 'fingerprint': 'native-ui-fixture', 'port': 53318, 'protocol': 'https'},
    'files': {'one': {'id': 'one', 'fileName': 'Native-UI-acceptance.txt', 'size': len(payload),
                      'fileType': 'text/plain', 'sha256': hashlib.sha256(payload).hexdigest()}}
}
prepare = urllib.request.Request(base + '/prepare-upload', data=json.dumps(request).encode(), headers={'Content-Type':'application/json'})
print('Waiting for native UI consent', flush=True)
with urllib.request.urlopen(prepare, context=context, timeout=125) as response:
    accepted = json.load(response)
query = urllib.parse.urlencode({'sessionId': accepted['sessionId'], 'fileId': 'one', 'token': accepted['files']['one']})
upload = urllib.request.Request(base + '/upload?' + query, data=payload, method='POST', headers={'Content-Type':'application/octet-stream'})
with urllib.request.urlopen(upload, context=context, timeout=20) as response:
    assert response.status == 200
folder = pathlib.Path.home() / 'Downloads/LinuxDrop'
matches = list(folder.glob('Native-UI-acceptance*.txt'))
assert any(path.read_bytes() == payload for path in matches), 'Received bytes differ'
print('PASS: native UI consent, HTTPS upload, exact saved bytes', flush=True)
