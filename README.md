# Lyra Downloads

Gerenciador de downloads para Linux com interface nativa **GTK4 +
libadwaita**, motor **aria2** e integração opcional com o **Firefox**. Parte
do ecossistema Lyra OS (foco em GNOME/Wayland e openSUSE Leap 16).

- HTTP e HTTPS, com até **1, 4, 8 ou 16 conexões** por download (é um máximo
  pedido ao servidor, não uma garantia de conexões nem de velocidade).
- Fila persistente: pausar, retomar, cancelar, tentar novamente, reordenar,
  limite de downloads simultâneos e de velocidade total.
- Continua baixando com a janela fechada; retoma após reiniciar o computador.
- Verificação opcional de **SHA-256** ao concluir.
- Velocidade total, conexões ativas, progresso e tempo restante reais — o que
  não é conhecido aparece como desconhecido.
- Extensão *Lyra Downloads Integration*: "Baixar com Lyra Downloads" no menu
  de links e, se você quiser, o Lyra como gerenciador padrão dos downloads do
  Firefox (desligado por padrão, configurável no botão da extensão).

Fora desta versão (roadmap): torrents/magnet, sites de vídeo, agendamento
avançado, outros navegadores, Flatpak.

## Dependências (verificadas no openSUSE Leap 16, x86_64)

| Uso | Pacotes (versão observada no repositório) |
|---|---|
| Execução | `aria2` (1.37.0), `gtk4` (4.18), `libadwaita` (1.7), `sqlite3` (3.53) |
| Build | `rust`/`cargo` (1.97; mínimo 1.92), `gcc`, `pkgconfig`, `gtk4-devel`, `libadwaita-devel`, `sqlite3-devel`, `gettext-tools`, `python3` |
| Extensão (só desenvolvimento) | `nodejs24`/npm; `typescript` e `web-ext` vêm do `package-lock.json` |
| Validação | `desktop-file-utils`, `appstream-glib`, `AppStream` (appstreamcli) |

```sh
sudo zypper install aria2 rust cargo gcc pkgconfig gtk4-devel libadwaita-devel \
    sqlite3-devel gettext-tools python3 desktop-file-utils appstream-glib nodejs24
```

Outras distribuições funcionam desde que ofereçam versões equivalentes;
isso não foi testado.

## Compilar e executar

```sh
cargo build --release --locked       # ou scripts/build-all.sh (inclui a extensão)
./target/release/lyra-downloads
```

A interface inicia sozinha o serviço em segundo plano (`lyra-downloads-backend`)
quando necessário; ele usa uma instância própria do aria2.

Instalação local para desenvolvimento (em `~/.local`, sem root; registra o
native host do Firefox só para o seu usuário):

```sh
scripts/install-local.sh
scripts/uninstall-local.sh                 # mantém histórico e downloads
scripts/uninstall-local.sh --remover-dados # apaga também ~/.local/share/lyra-downloads
```

Downloads nunca são apagados pelos scripts. Pacotes oficiais: RPM pelo Open
Build Service — veja [`packaging/obs/README-OBS.md`](packaging/obs/README-OBS.md).

## Onde ficam os dados

| Caminho | Conteúdo |
|---|---|
| `$XDG_DATA_HOME/lyra-downloads/lyra-downloads.db` | tarefas, histórico, preferências (SQLite) |
| `$XDG_DATA_HOME/lyra-downloads/aria2/` | sessão, configuração privada e log do aria2 |
| `$XDG_DATA_HOME/lyra-downloads/backend.log` | log do serviço (URLs redigidas) |
| `$XDG_RUNTIME_DIR/lyra-downloads/` | socket, lock de instância única, segredo RPC |

## Firefox

Resumo (detalhes em [`docs/FIREFOX.md`](docs/FIREFOX.md)):

```sh
scripts/install-native-host.sh            # registra o host para o seu usuário
cd extensions/firefox && npm ci && npm test && npx web-ext run --source-dir dist
```

O pacote gerado por `npm run package` **não é assinado** e serve só para
desenvolvimento. Distribuição para uso normal exige assinatura da Mozilla.
Firefox em Flatpak/Snap não é suportado nesta versão.

## Testes

```sh
scripts/check.sh
```

Executa `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`
(unitários + integração com **aria2c real** e um servidor HTTP local com
ranges/206, sem ranges, sem tamanho, redirecionamento, 403/404/conexão
recusada, pausa/retomada, reinício do backend, idempotência e o native host
morto no meio do download), conferência de traduções e licenças, validação do
`.desktop`/AppStream, testes TypeScript da extensão e `web-ext lint`. Nenhum
teste acessa a Internet. Sem `aria2c`, os testes de integração são pulados e
avisam que isso **não conta como aprovado**.

Servidor de teste manual: `cargo run -p lyra-downloads-testserver -- 127.0.0.1:8765`
(rotas em `tools/testserver/src/lib.rs`).

## Solução de problemas

| Sintoma | O que fazer |
|---|---|
| Faixa "aria2c não foi encontrado" | `sudo zypper install aria2`; ou aponte `LYRA_DOWNLOADS_ARIA2C=/caminho/aria2c`. |
| "O motor aria2 falhou várias vezes" | Veja `~/.local/share/lyra-downloads/aria2/aria2.log` e use "Tentar iniciar o motor". |
| Erro 403 | Pode ser link expirado, que exige login ou bloqueado; tente de novo ou "Tentar com um novo link". |
| Extensão mostra "Aplicativo indisponível" | Rode `scripts/install-native-host.sh` (ou instale `lyra-downloads-firefox-integration`) e confira `~/.mozilla/native-messaging-hosts/org.lyraos.downloads.json`. |
| Download ficou no Firefox mesmo com o Lyra como padrão | Esperado para janelas privadas, POST, autenticação, `blob:`/`data:`, downloads que não pausam e, por padrão, os que enviam cookies. O motivo aparece no console da extensão (`about:debugging`). |
| Logs detalhados | `LYRA_DOWNLOADS_LOG=debug lyra-downloads` (interface) / o backend grava em `backend.log`. |

## Documentação

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — componentes, ciclo de vida, autoridade dos dados, estados.
- [`docs/PROTOCOL.md`](docs/PROTOCOL.md) — IPC local, Native Messaging e estados de repasse.
- [`docs/FIREFOX.md`](docs/FIREFOX.md) — extensão, registro do host, assinatura.
- [`packaging/obs/README-OBS.md`](packaging/obs/README-OBS.md) — RPM/OBS.
- [`PLAN.md`](PLAN.md) — decisões e pendências.

## Tradução

As mensagens são escritas em português do Brasil. Para outro idioma:
`scripts/update-pot.py`, copie `po/lyra-downloads.pot` para `po/<idioma>.po`,
traduza e adicione o idioma a `po/LINGUAS`.

## Licença

GPL-3.0-or-later ([`LICENSE`](LICENSE)). Dependências Rust incorporadas:
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md). O ícone atual é
provisório, desenhado no estilo dos demais aplicativos Lyra, e não é um logo
oficial aprovado.
