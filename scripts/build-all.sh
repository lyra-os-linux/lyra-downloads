#!/usr/bin/env bash
# Constrói todos os componentes a partir dos lockfiles:
#   binários Rust (release) e catálogos de tradução. A extensão do Firefox
#   fica em github.com/lyra-os-linux/lyra-firefox-ext.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo build --release --locked -p lyra-downloads-gtk -p lyra-downloads-backend -p lyra-downloads-nativehost

for locale in $(grep -v '^#' po/LINGUAS); do
    msgfmt --check --check-format -o "po/$locale.mo" "po/$locale.po"
done

echo
echo "binários: target/release/{lyra-downloads,lyra-downloads-backend,lyra-downloads-nativehost}"
