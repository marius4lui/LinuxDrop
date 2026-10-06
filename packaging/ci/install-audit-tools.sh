#!/bin/sh
set -eu
destination=${1:?provide a private destination directory}
mkdir -p "$destination"
temporary=$(mktemp -d)
trap 'rm -rf -- "$temporary"' EXIT HUP INT TERM
fetch() {
    name=$1 url=$2 digest=$3
    curl --fail --location --proto '=https' --tlsv1.2 "$url" -o "$temporary/archive"
    printf '%s  %s\n' "$digest" "$temporary/archive" | sha256sum --check --status
    member=$(tar -tf "$temporary/archive" | awk -v name="$name" '$0 == name || $0 ~ ("/" name "$") {print; exit}')
    test -n "$member"
    tar -xOf "$temporary/archive" "$member" > "$destination/$name"
    chmod 755 "$destination/$name"
}
fetch cargo-audit https://github.com/rustsec/rustsec/releases/download/cargo-audit/v0.22.2/cargo-audit-x86_64-unknown-linux-musl-v0.22.2.tgz 7fb9497f8594b389e5fce5ef9b92db08432996895b2e0c5a0167a69ed445c428
fetch cargo-deny https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz 9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f
