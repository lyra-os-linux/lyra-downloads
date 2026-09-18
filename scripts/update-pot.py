#!/usr/bin/python3
"""Extrai as mensagens tr()/trf() da interface para po/lyra-downloads.pot.

O xgettext 0.22 não tem analisador de Rust e o analisador de C interpreta mal
lifetimes como `&'static`, corrompendo textos acentuados; por isso este
extrator simples (as chamadas usam sempre um literal de string como 1º argumento).
Com --check, falha se o .pot versionado estiver desatualizado ou se algum
catálogo em po/LINGUAS não cobrir todas as mensagens.
"""
from __future__ import annotations

import ast
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
POT = ROOT / "po" / "lyra-downloads.pot"
CALL = re.compile(r'\btrf?\(\s*"((?:[^"\\]|\\.)*)"')


def extract() -> dict[str, list[str]]:
    messages: dict[str, list[str]] = {}
    for rel in (ROOT / "po" / "POTFILES.in").read_text(encoding="utf-8").split():
        text = (ROOT / rel).read_text(encoding="utf-8")
        for m in CALL.finditer(text):
            line = text.count("\n", 0, m.start()) + 1
            msg = ast.literal_eval(f'"{m.group(1)}"')
            messages.setdefault(msg, []).append(f"{rel}:{line}")
    return messages


def po_quote(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def render(messages: dict[str, list[str]]) -> str:
    out = [
        "# Modelo de tradução do Lyra Downloads.",
        "# Idioma-fonte: português do Brasil.",
        "#, fuzzy",
        'msgid ""',
        'msgstr ""',
        '"Project-Id-Version: lyra-downloads\\n"',
        '"Report-Msgid-Bugs-To: https://github.com/lyra-os-linux/lyra-downloads/issues\\n"',
        '"PO-Revision-Date: YEAR-MO-DA HO:MI+ZONE\\n"',
        '"Last-Translator: FULL NAME <EMAIL@ADDRESS>\\n"',
        '"Language-Team: LANGUAGE <LL@li.org>\\n"',
        '"Language: \\n"',
        '"MIME-Version: 1.0\\n"',
        '"Content-Type: text/plain; charset=UTF-8\\n"',
        '"Content-Transfer-Encoding: 8bit\\n"',
        "",
    ]
    for msg in sorted(messages, key=lambda m: messages[m][0]):
        out.append("#: " + " ".join(messages[msg]))
        if "{" in msg:
            out.append("#, placeholders mantidos entre chaves")
        out.append("msgid " + po_quote(msg))
        out.append('msgstr ""')
        out.append("")
    return "\n".join(out)


def main() -> int:
    messages = extract()
    content = render(messages)
    if "--check" in sys.argv:
        # Compara só o conjunto de mensagens (referências de linha mudam
        # com qualquer formatação e não afetam a tradução).
        current = set()
        if POT.exists():
            for line in POT.read_text(encoding="utf-8").splitlines():
                if line.startswith("msgid "):
                    current.add(ast.literal_eval(line[6:]))
        current.discard("")
        if current != set(messages):
            missing = sorted(set(messages) - current)
            extra = sorted(current - set(messages))
            print(f"po/lyra-downloads.pot desatualizado (faltam {missing[:3]}…, sobram {extra[:3]}…): "
                  "rode scripts/update-pot.py", file=sys.stderr)
            return 1
    else:
        POT.write_text(content, encoding="utf-8")
    with tempfile.NamedTemporaryFile(suffix=".mo") as mo:
        subprocess.run(["msgfmt", "--check-format", "-o", mo.name, str(POT)], check=True)
    linguas = [
        line.strip()
        for line in (ROOT / "po" / "LINGUAS").read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    for locale in linguas:
        po = ROOT / "po" / f"{locale}.po"
        subprocess.run(["msgmerge", "--quiet", "--update", "--backup=none", str(po), str(POT)], check=True)
        with tempfile.NamedTemporaryFile(suffix=".mo") as mo:
            subprocess.run(["msgfmt", "--check", "--check-format", "-o", mo.name, str(po)], check=True)
    print(f"{POT.relative_to(ROOT)}: {content.count(chr(10) + 'msgid ')} mensagens")
    return 0


if __name__ == "__main__":
    sys.exit(main())
