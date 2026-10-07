#!/usr/bin/env python3
"""Keep the GNOME schema generated from the real Rust status models."""
import argparse
import json
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
result = subprocess.run(['cargo', 'run', '--quiet', '--locked', '-p', 'linuxdrop-ipc', '--features', 'schema', '--example', 'export_schema'], cwd=root, text=True, capture_output=True, check=True)
schema = json.loads(result.stdout)
serialized = json.dumps(schema, indent=2, ensure_ascii=False) + '\n'
outputs = {'crates/linuxdrop-ipc/snapshot.schema.json': serialized, 'extensions/gnome-shell/snapshot-schema.js': '// Generated from Rust by tools/sync-ipc-schema.py. Do not edit.\nexport const snapshotSchema = ' + serialized.rstrip() + ';\n'}
for name, content in outputs.items():
    path = root / name
    if args.check:
        if not path.exists() or path.read_text(encoding='utf-8') != content:
            raise SystemExit(f'Stale status schema: {name}; run tools/sync-ipc-schema.py')
    else:
        path.write_text(content, encoding='utf-8')
print('Shared status schema ' + ('verified' if args.check else 'generated'))
