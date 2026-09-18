#!/usr/bin/python3
"""Licenças das crates distribuídas (ligadas aos binários do RPM).

Gera THIRD_PARTY_LICENSES.md a partir de `cargo metadata` (dependências
normais alcançáveis a partir de lyra-downloads-gtk, -backend e
-nativehost, em x86_64 Linux) e confere se o campo License do spec cobre
todas as expressões encontradas.

  scripts/third-party-licenses.py          # regenera o arquivo
  scripts/third-party-licenses.py --check  # falha se estiver desatualizado
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "THIRD_PARTY_LICENSES.md"
SPEC = ROOT / "packaging" / "obs" / "lyra-downloads.spec"
ROOTS = {"lyra-downloads-gtk", "lyra-downloads-backend", "lyra-downloads-nativehost"}


def normalize(expr: str) -> str:
    # Formato antigo "MIT/Apache-2.0" equivale a "MIT OR Apache-2.0".
    return " OR ".join(p.strip() for p in expr.split("/")) if "/" in expr else expr


def crates() -> list[tuple[str, str, str]]:
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline",
         "--filter-platform", "x86_64-unknown-linux-gnu"],
        cwd=ROOT, check=True, capture_output=True, text=True).stdout)
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    ws = set(meta["workspace_members"])
    stack = [i for i in ws if pkgs[i]["name"] in ROOTS]
    seen: set[str] = set()
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            # Só dependências normais são ligadas aos binários; dependências
            # de build (scripts build.rs, geradores) não são distribuídas.
            if any(k["kind"] is None for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    return sorted(
        (pkgs[i]["name"], pkgs[i]["version"], normalize(pkgs[i]["license"] or "DESCONHECIDA"))
        for i in seen if i not in ws
    )


def render(items: list[tuple[str, str, str]]) -> str:
    lines = [
        "# Licenças de terceiros",
        "",
        "Crates Rust incorporadas aos binários distribuídos do Lyra Downloads",
        "(gerado por `scripts/third-party-licenses.py`; não editar à mão).",
        "O texto de cada licença acompanha a respectiva crate no tarball",
        "`vendor.tar.zst`.",
        "",
        "| Crate | Versão | Licença (SPDX) |",
        "|---|---|---|",
    ]
    lines += [f"| {n} | {v} | {l} |" for n, v, l in items]
    return "\n".join(lines) + "\n"


def canon(term: str) -> str:
    """Forma canônica de um termo sem AND: operandos de OR em ordem."""
    return " OR ".join(sorted(p.strip() for p in term.strip().strip("()").split(" OR ")))


def terms(expr: str) -> set[str]:
    return {canon(t) for t in expr.split(" AND ")}


def spec_terms() -> set[str]:
    m = re.search(r"^License:\s*(.+)$", SPEC.read_text(encoding="utf-8"), re.M)
    return terms(m.group(1)) if m else set()


def main() -> int:
    items = crates()
    content = render(items)
    exprs = {l for _, _, l in items}
    # Toda expressão (ou cada termo de um "A AND B") precisa aparecer no spec.
    needed: set[str] = set()
    for e in exprs:
        needed |= terms(e)
    missing = sorted(needed - spec_terms())
    if "DESCONHECIDA" in exprs:
        print("crate sem licença declarada; revise manualmente", file=sys.stderr)
        return 1
    if "--check" in sys.argv:
        ok = OUT.exists() and OUT.read_text(encoding="utf-8") == content
        if not ok:
            print("THIRD_PARTY_LICENSES.md desatualizado: rode scripts/third-party-licenses.py", file=sys.stderr)
        if missing:
            print("License do spec não cobre: " + "; ".join(missing), file=sys.stderr)
        return 0 if ok and not missing else 1
    OUT.write_text(content, encoding="utf-8")
    print(f"{len(items)} crates; expressões distintas:")
    for e in sorted(exprs):
        print("  " + e)
    if missing:
        print("ATENÇÃO — incluir no License do spec: " + "; ".join(missing))
    return 0


if __name__ == "__main__":
    sys.exit(main())
