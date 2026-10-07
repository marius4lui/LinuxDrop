#!/usr/bin/env python3
"""Client-side real Polkit probe, run in a disposable PAM/logind session."""
import json
import os
import select
import socket
import subprocess
import sys

agent = None
try:
    session = os.environ.get('XDG_SESSION_ID')
    metadata = subprocess.check_output(['loginctl', 'show-session', session,
        '-p', 'Active', '-p', 'Remote', '-p', 'Seat', '-p', 'VTNr', '-p', 'Leader'], text=True)
    details = dict(line.split('=', 1) for line in metadata.splitlines())
    assert details['Leader'] == str(os.getpid()), 'Client must lead its actual PAM session'
    print('LINUXDROP_AUTH_SESSION:' + json.dumps(details), flush=True)
    subprocess.run(['pkcheck', '--revoke-temp'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    if '--agent' in sys.argv:
        read_fd, write_fd = os.pipe()
        agent = subprocess.Popen(['pkttyagent', '--process', str(os.getpid()), '--notify-fd', str(write_fd)], pass_fds=(write_fd,))
        os.close(write_fd)
        if not select.select([read_fd], [], [], 10)[0]:
            raise RuntimeError('Agent registration timed out')
        os.read(read_fd, 1)
        os.close(read_fd)
        if agent.poll() is not None:
            raise RuntimeError('Agent could not register')
    print('LINUXDROP_AUTH_CLIENT_READY', flush=True)
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(65)
        client.connect('/run/linuxdrop/netd.sock')
        client.sendall(b'{"operation":"reserve","radio_id":"acceptance-no-radio"}\n')
        response = bytearray()
        while not response.endswith(b'\n'):
            chunk = client.recv(4096)
            if not chunk:
                raise RuntimeError('Helper closed before authorization result')
            response.extend(chunk)
            if len(response) > 65536:
                raise RuntimeError('Oversized helper response')
    print('LINUXDROP_AUTH_RESULT:' + json.dumps(json.loads(response)), flush=True)
finally:
    if agent is not None:
        agent.terminate()
        try: agent.wait(timeout=2)
        except subprocess.TimeoutExpired:
            agent.kill(); agent.wait()
