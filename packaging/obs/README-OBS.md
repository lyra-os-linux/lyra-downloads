# Empacotamento no Open Build Service

Pacotes gerados a partir de `lyra-downloads.spec`:

| Pacote | Conteúdo |
|---|---|
| `lyra-downloads` | interface, backend, `.desktop`, ícones, AppStream |
| `lyra-downloads-firefox-integration` | native host + registro global do Firefox; `Requires: lyra-downloads = versão-release`; não é `noarch` |

Alvo: **openSUSE Leap 16.0, x86_64** (`ExclusiveArch: x86_64`). O spec foi
escrito contra os pacotes do Leap 16 (rust/cargo 1.97, gtk4 4.18, libadwaita
1.7, aria2 1.37, `cargo-packaging` 1.2). Tumbleweed deve funcionar com o mesmo
spec, mas **não foi validado**; não habilite outros alvos/arquiteturas sem
build e teste.

> Projeto OBS, conta, URL Git de lançamento e credenciais **não foram
> definidos**. Os comandos abaixo usam `<PROJETO_OBS>` e `<URL_DO_REPO>` como
> parâmetros explícitos. Nada foi publicado.

## 1. Preparar as fontes (etapa com rede, fora do build)

```sh
packaging/obs/make-sources.sh v0.1.0      # revisão/tag identificável (padrão: HEAD)
# resultado em packaging/obs/out/:
#   lyra-downloads-0.1.0.tar.zst   git archive da revisão (.source-revision dentro)
#   vendor.tar.zst                 cargo vendor --locked + .cargo/config.toml
#   lyra-downloads.spec, lyra-downloads.changes, SHA256SUMS
```

Os tarballs são determinísticos: `SOURCE_DATE_EPOCH` = data do commit, ordem
de arquivos e donos fixos. Refazer com a mesma revisão e o mesmo `Cargo.lock`
produz os mesmos arquivos.

`--worktree` empacota a árvore de trabalho atual — só para testes locais.

Alternativa no OBS: enviar só o tarball de fontes e gerar `vendor.tar.zst`
com o serviço `cargo_vendor` (`_service`, modo manual, `respect-lockfile`):

```sh
osc service manualrun
```

Quando existir uma tag publicada, `packaging/obs/gen-service.sh <URL_DO_REPO>
<tag>` gera um `_service` com `obs_scm` a partir do modelo (confere a tag no
remoto antes). Esse fluxo com `obs_scm` **ainda não foi testado**.

## 2. Build sem rede

`%prep`, `%build`, `%install` e `%check` não acessam a Internet:

- `%autosetup -a1` extrai as crates vendorizadas e o `.cargo/config.toml` que
  aponta para elas.
- `%{cargo_build} --locked` (macro do `cargo-packaging`: `--offline --release`,
  `cargo auditable`, debuginfo preservado para o `debuginfo`/`debugsource`).
- Nada de `rustup`, `cargo install`, `curl` ou `npm` no spec. A extensão do
  Firefox é outro pacote (`lyra-firefox-ext`); o RPM não requer Node.
- `%check`: validação do `.desktop` e do AppStream, conferência do catálogo
  de tradução e da lista de licenças, e `cargo test --offline`. Os testes de
  integração sobem `aria2c` e um servidor HTTP em `127.0.0.1`, em diretórios
  temporários; por isso `aria2` é `BuildRequires`.

O `aria2` é dependência de runtime do repositório oficial; nada é baixado no
primeiro uso.

## 3. Build local

Com `osc` configurado para um projeto seu:

```sh
osc checkout <PROJETO_OBS>/lyra-downloads   # ou: osc mkpac lyra-downloads
cp packaging/obs/out/* <checkout>/
cd <checkout>
osc build openSUSE_Leap_16.0 x86_64
```

Sem OBS, um build com `rpmbuild` também serve para validar o spec (exige as
`BuildRequires` instaladas):

```sh
mkdir -p ~/rpmbuild/SOURCES && cp packaging/obs/out/*.tar.zst ~/rpmbuild/SOURCES/
rpmbuild -ba packaging/obs/lyra-downloads.spec
```

## 4. Validação

```sh
rpmlint ~/rpmbuild/RPMS/x86_64/lyra-downloads*.rpm
rpm -qlp ~/rpmbuild/RPMS/x86_64/lyra-downloads-firefox-integration-*.rpm
rpm -qp --requires ~/rpmbuild/RPMS/x86_64/lyra-downloads-firefox-integration-*.rpm
```

Conferir: o manifesto em
`/usr/lib64/mozilla/native-messaging-hosts/org.lyraos.downloads.json` aponta
para `/usr/bin/lyra-downloads-nativehost`; nenhum arquivo em `/home` ou
caminho da máquina de desenvolvimento; os scriptlets só atualizam caches de
ícones/desktop (não criam dados de usuário nem mexem em perfis do Firefox).
Dados e downloads dos usuários não são tocados em atualização ou remoção; o
banco é migrado pelo próprio backend, no contexto do usuário.

## 5. Nova versão

1. Atualizar `version` em `Cargo.toml` (workspace).
2. `cargo update -p …` se necessário; `scripts/third-party-licenses.py` e
   ajustar `License:` do spec se aparecerem novas licenças.
3. Entrada no topo de `lyra-downloads.changes` (`osc vc`) e `Version:` no spec.
4. Tag Git, `make-sources.sh <tag>`, `osc add/commit`.

## 6. Instalar via zypper (depois de publicado)

```sh
sudo zypper addrepo --refresh <URL_DO_REPOSITORIO_OBS> lyra-downloads
# ex. de formato: https://download.opensuse.org/repositories/<PROJETO_OBS_COM_BARRAS>/openSUSE_Leap_16.0/
sudo zypper install lyra-downloads lyra-downloads-firefox-integration
```

A assinatura do repositório/RPM (chave do projeto OBS) e a assinatura da
extensão do Firefox (Mozilla) são processos distintos.
