#!/usr/bin/env bash
# Remove apenas o registro do native host do usuário atual.
set -euo pipefail
dest="$HOME/.mozilla/native-messaging-hosts/org.lyraos.downloads.json"
if [[ -f "$dest" ]]; then
    rm -f -- "$dest"
    echo "registro removido: $dest"
else
    echo "nenhum registro do usuário encontrado em $dest"
fi
