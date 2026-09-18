#!/usr/bin/env bash
# Gera packaging/obs/_service a partir do modelo obs_scm, para uma URL Git e
# uma tag REAIS. Confere que a tag existe no remoto antes de escrever nada.
# Uso: packaging/obs/gen-service.sh https://github.com/<org>/lyra-downloads.git v0.1.0
set -euo pipefail
url="${1:?informe a URL do repositório Git}"
tag="${2:?informe a tag de lançamento (ex.: v0.1.0)}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! git ls-remote --exit-code --tags "$url" "refs/tags/$tag" >/dev/null; then
    echo "erro: a tag $tag não existe em $url" >&2
    exit 1
fi
python3 - "$here/_service.obs_scm.template" "$here/_service" "$url" "$tag" <<'EOF'
import re, sys
template, out, url, tag = sys.argv[1:]
text = open(template, encoding="utf-8").read()
text = re.sub(r"<!--.*?-->\n", "", text, count=1, flags=re.S)
text = text.replace("@GIT_URL@", url).replace("@REVISION@", tag)
open(out, "w", encoding="utf-8").write(text)
EOF
echo "_service gerado para $url @ $tag"
