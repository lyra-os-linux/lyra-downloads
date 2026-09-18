#!/usr/bin/env bash
# Constrói todos os componentes a partir dos lockfiles:
#   binários Rust (release), catálogos de tradução e o pacote NÃO assinado
#   da extensão do Firefox (extensions/firefox/artifacts/).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo build --release --locked -p lyra-downloads-gtk -p lyra-downloads-backend -p lyra-downloads-nativehost

for locale in $(grep -v '^#' po/LINGUAS); do
    msgfmt --check --check-format -o "po/$locale.mo" "po/$locale.po"
done

(
    cd extensions/firefox
    npm ci --no-audit --no-fund
    npm run package
)

echo
echo "binários: target/release/{lyra-downloads,lyra-downloads-backend,lyra-downloads-nativehost}"
echo "extensão (não assinada, só desenvolvimento): extensions/firefox/artifacts/"
