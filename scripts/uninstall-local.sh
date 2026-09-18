#!/usr/bin/env bash
# Desfaz scripts/install-local.sh. Por padrão PRESERVA os dados do
# aplicativo (histórico, fila, sessão do aria2) e NUNCA apaga arquivos
# baixados. `--remover-dados` apaga apenas ~/.local/share/lyra-downloads.
set -euo pipefail

prefix="${PREFIX:-$HOME/.local}"
remove_data=false
[[ "${1:-}" == "--remover-dados" ]] && remove_data=true

socket="${XDG_RUNTIME_DIR:-/nonexistent}/lyra-downloads/backend.sock"
if [[ -S "$socket" ]]; then
    # Pede ao backend para pausar tudo e encerrar de forma limpa.
    python3 - "$socket" <<'EOF' || true
import json, socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(30)
s.connect(sys.argv[1])
s.sendall(b'{"v":1,"request_id":"uninstall","op":"shutdown"}\n')
s.makefile().readline()
EOF
fi

rm -f -- "$prefix/bin/lyra-downloads" "$prefix/bin/lyra-downloads-backend" "$prefix/bin/lyra-downloads-nativehost" \
    "$prefix/share/applications/org.lyraos.Downloads.desktop" \
    "$prefix/share/metainfo/org.lyraos.Downloads.metainfo.xml" \
    "$prefix/share/icons/hicolor/scalable/apps/org.lyraos.Downloads.svg" \
    "$prefix/share/icons/hicolor/symbolic/apps/org.lyraos.Downloads-symbolic.svg"
find "$prefix/share/locale" -path '*/LC_MESSAGES/lyra-downloads.mo' -delete 2>/dev/null || true
"$(dirname "${BASH_SOURCE[0]}")/uninstall-native-host.sh"

data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/lyra-downloads"
if $remove_data; then
    rm -rf -- "$data_dir"
    echo "dados do aplicativo removidos ($data_dir). Os arquivos baixados foram mantidos."
else
    echo "dados preservados em $data_dir (use --remover-dados para apagá-los). Arquivos baixados não são tocados."
fi
