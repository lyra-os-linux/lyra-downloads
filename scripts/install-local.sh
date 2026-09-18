#!/usr/bin/env bash
# Instalação local para desenvolvimento, só para o usuário atual (~/.local).
# Não usa root, não mexe em perfis do Firefox e não toca em downloads nem
# em dados do usuário (banco/sessão em ~/.local/share/lyra-downloads).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
prefix="${PREFIX:-$HOME/.local}"

cargo build --release --locked --workspace --bins

install -Dm0755 target/release/lyra-downloads "$prefix/bin/lyra-downloads"
install -Dm0755 target/release/lyra-downloads-backend "$prefix/bin/lyra-downloads-backend"
install -Dm0755 target/release/lyra-downloads-nativehost "$prefix/bin/lyra-downloads-nativehost"

# Exec absoluto: ~/.local/bin nem sempre está no PATH da sessão gráfica.
desktop="$prefix/share/applications/org.lyraos.Downloads.desktop"
install -Dm0644 data/org.lyraos.Downloads.desktop "$desktop"
sed -i "s|^Exec=lyra-downloads$|Exec=$prefix/bin/lyra-downloads|" "$desktop"
install -Dm0644 data/org.lyraos.Downloads.metainfo.xml "$prefix/share/metainfo/org.lyraos.Downloads.metainfo.xml"
install -Dm0644 data/icons/org.lyraos.Downloads.svg "$prefix/share/icons/hicolor/scalable/apps/org.lyraos.Downloads.svg"
install -Dm0644 data/icons/org.lyraos.Downloads-symbolic.svg "$prefix/share/icons/hicolor/symbolic/apps/org.lyraos.Downloads-symbolic.svg"

if [[ -f po/LINGUAS ]]; then
    for locale in $(grep -v '^#' po/LINGUAS); do
        mkdir -p "$prefix/share/locale/$locale/LC_MESSAGES"
        msgfmt --check -o "$prefix/share/locale/$locale/LC_MESSAGES/lyra-downloads.mo" "po/$locale.po"
    done
fi

command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
command -v gtk4-update-icon-cache >/dev/null && gtk4-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true

"$repo_root/scripts/install-native-host.sh" "$prefix/bin/lyra-downloads-nativehost"

echo
echo "Lyra Downloads instalado em $prefix."
echo "A extensão do Firefox é instalada à parte (veja docs/FIREFOX.md)."
