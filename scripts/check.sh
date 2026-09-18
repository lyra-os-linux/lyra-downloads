#!/usr/bin/env bash
# Todas as verificações locais. Sai com erro na primeira falha.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '\n==> %s\n' "$*"; }

step "cargo fmt --check"
cargo fmt --all -- --check
step "cargo clippy"
cargo clippy --locked --workspace --all-targets -- -D warnings
step "cargo test (inclui integração com aria2c e servidor HTTP locais)"
cargo test --locked --workspace
step "catálogo de tradução e licenças de terceiros"
python3 scripts/update-pot.py --check
python3 scripts/third-party-licenses.py --check
step ".desktop e AppStream"
desktop-file-validate data/org.lyraos.Downloads.desktop
appstreamcli validate --no-net data/org.lyraos.Downloads.metainfo.xml
appstream-util validate-relax --nonet data/org.lyraos.Downloads.metainfo.xml
step "scripts shell"
bash -n scripts/*.sh packaging/obs/*.sh
echo
echo "todas as verificações passaram"
