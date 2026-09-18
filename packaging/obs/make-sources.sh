#!/usr/bin/env bash
# Gera as fontes para o OBS a partir de uma revisão Git identificável:
#   lyra-downloads-<versão>.tar.zst  (git archive da revisão)
#   vendor.tar.zst                   (crates do Cargo.lock + .cargo/config.toml)
#
# O build RPM usa só esses arquivos, sem rede. Esta preparação é uma etapa
# separada (precisa de rede para baixar crates) e é determinística: mtimes,
# donos e ordem dos arquivos vêm da revisão (SOURCE_DATE_EPOCH = data do commit).
#
# Uso:
#   packaging/obs/make-sources.sh [REVISÃO]        # padrão: HEAD
#   packaging/obs/make-sources.sh --worktree       # SÓ para teste local: usa a
#       árvore de trabalho atual (arquivos versionados + não ignorados); o
#       resultado não corresponde a nenhuma revisão e não deve ser publicado.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${OUT_DIR:-$repo_root/packaging/obs/out}"
cd "$repo_root"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
name="lyra-downloads-$version"
mode="${1:-HEAD}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$out_dir" "$work/$name"

if [[ "$mode" == "--worktree" ]]; then
    echo "AVISO: fontes da árvore de trabalho (não identificáveis); só para teste local." >&2
    export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(date +%s)}"
    git ls-files -z --cached --others --exclude-standard \
        | tar --null --ignore-failed-read -T - -cf - 2>/dev/null | tar -xf - -C "$work/$name"
else
    rev="$(git rev-parse --verify "$mode^{commit}")"
    export SOURCE_DATE_EPOCH="$(git log -1 --format=%ct "$rev")"
    if [[ -n "$(git status --porcelain)" ]]; then
        echo "aviso: há alterações não commitadas; elas NÃO entram no tarball de $rev" >&2
    fi
    git archive --format=tar "$rev" | tar -xf - -C "$work/$name"
    echo "$rev" > "$work/$name/.source-revision"
fi

deterministic_tar() { # $1 = arquivo de saída, $2 = diretório base, $3.. = entradas
    local outfile="$1" base="$2"; shift 2
    tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner \
        --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
        -C "$base" -cf - "$@" | zstd -q -19 -T0 --no-progress -o "$outfile" -f
}

# Fontes do projeto.
deterministic_tar "$out_dir/$name.tar.zst" "$work" "$name"

# Crates vendorizadas exatamente conforme o Cargo.lock (--locked).
(
    cd "$work/$name"
    mkdir -p .cargo
    cargo vendor --locked --versioned-dirs vendor > .cargo/config.toml
)
deterministic_tar "$out_dir/vendor.tar.zst" "$work/$name" vendor .cargo/config.toml

cp packaging/obs/lyra-downloads.spec packaging/obs/lyra-downloads.changes packaging/obs/lyra-downloads-rpmlintrc "$out_dir/"
( cd "$out_dir" && sha256sum "$name.tar.zst" vendor.tar.zst > SHA256SUMS )
echo "fontes em $out_dir:"
ls -l "$out_dir"
