#
# spec file for package lyra-downloads
#
# Copyright (c) 2026 Rodrigo Brito
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#

Name:           lyra-downloads
Version:        0.1.0
Release:        0
Summary:        Download manager with parallel connections for the Lyra OS ecosystem
# Código do projeto: GPL-3.0-or-later. Crates vendorizadas (ligadas
# estaticamente aos binários) — conjunto gerado por
# scripts/third-party-licenses.py; lista completa em THIRD_PARTY_LICENSES.md.
License:        GPL-3.0-or-later AND MIT AND Apache-2.0 AND ISC AND BSD-3-Clause AND MPL-2.0 AND Unicode-3.0 AND (Apache-2.0 OR MIT) AND (Apache-2.0 OR BSL-1.0) AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) AND (Apache-2.0 OR BSD-2-Clause OR MIT) AND (Apache-2.0 OR MIT OR Zlib) AND (MIT OR Unlicense)
Group:          Productivity/Networking/Other
URL:            https://github.com/lyra-os-linux/lyra-downloads
Source0:        %{name}-%{version}.tar.zst
Source1:        vendor.tar.zst
Source99:       %{name}-rpmlintrc
BuildRequires:  cargo
BuildRequires:  cargo-packaging
BuildRequires:  rust >= 1.92
BuildRequires:  gcc
BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(gtk4) >= 4.12
BuildRequires:  pkgconfig(libadwaita-1) >= 1.5
BuildRequires:  pkgconfig(sqlite3)
BuildRequires:  desktop-file-utils
BuildRequires:  appstream-glib
BuildRequires:  gettext-tools
BuildRequires:  python3
BuildRequires:  zstd
# Os testes de integração em %%check usam o aria2c real em loopback.
BuildRequires:  aria2
Requires:       aria2
ExclusiveArch:  x86_64

%description
Lyra Downloads is a download manager for HTTP and HTTPS with a native
GTK4/libadwaita interface, using aria2 as the download engine. Each download
can request up to 1, 4, 8 or 16 parallel connections (a maximum, not a
guarantee), with a persistent queue, pause/resume, recovery after restart and
optional SHA-256 verification.

O Lyra Downloads baixa arquivos por HTTP e HTTPS usando o aria2, com várias
conexões por download quando o servidor permite, fila persistente,
retomada após reiniciar e verificação opcional de SHA-256.

%package firefox-integration
Summary:        Firefox native messaging host for Lyra Downloads
Group:          Productivity/Networking/Other
Requires:       %{name} = %{version}
Enhances:       MozillaFirefox

%description firefox-integration
Native messaging host and its Firefox registration, used by the
"Lyra Downloads Integration" extension to send links and downloads to
Lyra Downloads.

This package does NOT install the Firefox extension itself: the extension
must be installed from a Mozilla-signed package. Lyra Downloads works
without this package.

%prep
# -a1 extrai Source1 (vendor.tar.zst: crates + .cargo/config.toml apontando
# para elas) sobre as fontes; o build não acessa a rede.
%autosetup -a1 -p1

%build
%{cargo_build} --locked \
    -p lyra-downloads-gtk -p lyra-downloads-backend -p lyra-downloads-nativehost
for locale in $(grep -v '^#' po/LINGUAS); do
    msgfmt --check --check-format -o po/${locale}.mo po/${locale}.po
done
python3 - packaging/native-host/org.lyraos.downloads.json.in \
    org.lyraos.downloads.json %{_bindir}/lyra-downloads-nativehost <<'EOF'
import json, sys
template, out, host = sys.argv[1:]
data = json.loads(open(template, encoding="utf-8").read())
data["path"] = host
# ID técnico de DESENVOLVIMENTO; trocar pelo ID definitivo da extensão
# assinada antes de publicar (ver docs/FIREFOX.md).
data["allowed_extensions"] = ["lyra-downloads@lyraos.com.br"]
open(out, "w", encoding="utf-8").write(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
EOF

%install
install -Dm0755 target/release/lyra-downloads %{buildroot}%{_bindir}/lyra-downloads
install -Dm0755 target/release/lyra-downloads-backend %{buildroot}%{_bindir}/lyra-downloads-backend
install -Dm0755 target/release/lyra-downloads-nativehost %{buildroot}%{_bindir}/lyra-downloads-nativehost
install -Dm0644 data/org.lyraos.Downloads.desktop \
    %{buildroot}%{_datadir}/applications/org.lyraos.Downloads.desktop
install -Dm0644 data/org.lyraos.Downloads.metainfo.xml \
    %{buildroot}%{_datadir}/metainfo/org.lyraos.Downloads.metainfo.xml
install -Dm0644 data/icons/org.lyraos.Downloads.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/org.lyraos.Downloads.svg
install -Dm0644 data/icons/org.lyraos.Downloads-symbolic.svg \
    %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/org.lyraos.Downloads-symbolic.svg
for locale in $(grep -v '^#' po/LINGUAS); do
    install -Dm0644 po/${locale}.mo \
        %{buildroot}%{_datadir}/locale/${locale}/LC_MESSAGES/lyra-downloads.mo
done
# Firefox de 64 bits do openSUSE procura manifestos globais em
# /usr/lib64/mozilla/native-messaging-hosts (verificado no Leap 16 x86_64,
# onde outros hosts, como o gnome-browser-connector, já são registrados).
# Não derivar isso de %%{_libdir} em outras arquiteturas sem verificar.
install -Dm0644 org.lyraos.downloads.json \
    %{buildroot}/usr/lib64/mozilla/native-messaging-hosts/org.lyraos.downloads.json
%find_lang %{name} || touch %{name}.lang

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/org.lyraos.Downloads.desktop
appstream-util validate-relax --nonet \
    %{buildroot}%{_datadir}/metainfo/org.lyraos.Downloads.metainfo.xml
python3 scripts/update-pot.py --check
python3 scripts/third-party-licenses.py --check
# Testes offline. Os de integração sobem aria2c e um servidor HTTP locais
# (127.0.0.1) e usam diretórios temporários; nada acessa a Internet.
%{cargo_test} --locked --workspace

%files -f %{name}.lang
%license LICENSE THIRD_PARTY_LICENSES.md
%doc README.md docs/ARCHITECTURE.md docs/PROTOCOL.md
%{_bindir}/lyra-downloads
%{_bindir}/lyra-downloads-backend
%{_datadir}/applications/org.lyraos.Downloads.desktop
%{_datadir}/metainfo/org.lyraos.Downloads.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/org.lyraos.Downloads.svg
%{_datadir}/icons/hicolor/symbolic/apps/org.lyraos.Downloads-symbolic.svg

%files firefox-integration
%license LICENSE
%doc docs/FIREFOX.md
%{_bindir}/lyra-downloads-nativehost
%dir /usr/lib64/mozilla
%dir /usr/lib64/mozilla/native-messaging-hosts
/usr/lib64/mozilla/native-messaging-hosts/org.lyraos.downloads.json

%changelog
