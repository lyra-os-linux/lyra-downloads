#!/usr/bin/env bash
# Registra o native host do Lyra Downloads para o Firefox DO USUÁRIO ATUAL
# (~/.mozilla/native-messaging-hosts). Não mexe em perfis do Firefox nem
# precisa de root. O RPM instala um registro global separado.
#
# Uso: scripts/install-native-host.sh [CAMINHO_DO_HOST] [ID_DA_EXTENSAO]
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
default_id="lyra-downloads@lyraos.com.br"

host_path="${1:-}"
if [[ -z "$host_path" ]]; then
    for candidate in "$HOME/.local/bin/lyra-downloads-nativehost" \
                     "$repo_root/target/release/lyra-downloads-nativehost" \
                     "$repo_root/target/debug/lyra-downloads-nativehost"; do
        if [[ -x "$candidate" ]]; then host_path="$candidate"; break; fi
    done
fi
if [[ -z "$host_path" || ! -x "$host_path" ]]; then
    echo "erro: executável lyra-downloads-nativehost não encontrado; compile antes ou informe o caminho." >&2
    exit 1
fi
host_path="$(realpath "$host_path")"
extension_id="${2:-$default_id}"
if [[ ! "$extension_id" =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+$ ]]; then
    echo "erro: ID de extensão inválido: $extension_id" >&2
    exit 1
fi

dest_dir="$HOME/.mozilla/native-messaging-hosts"
mkdir -p "$dest_dir"
dest="$dest_dir/org.lyraos.downloads.json"
tmp="$(mktemp "$dest_dir/.org.lyraos.downloads.XXXXXX")"
# Substituição literal (sem interpretar o conteúdo como expressão regular).
python3 - "$repo_root/packaging/native-host/org.lyraos.downloads.json.in" "$tmp" "$host_path" "$extension_id" <<'EOF'
import json, sys
template, out, host, ext = sys.argv[1:]
data = json.loads(open(template, encoding="utf-8").read())
data["path"] = host
data["allowed_extensions"] = [ext]
open(out, "w", encoding="utf-8").write(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
EOF
chmod 0644 "$tmp"
mv -f "$tmp" "$dest"
echo "native host registrado em $dest"
echo "  executável: $host_path"
echo "  extensão permitida: $extension_id"
