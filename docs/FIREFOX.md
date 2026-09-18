# Integração com o Firefox

A integração tem duas partes independentes:

1. **Native host** (`lyra-downloads-nativehost`) + registro
   `org.lyraos.downloads.json`. Vem no RPM `lyra-downloads-firefox-integration`
   (registro global em `/usr/lib64/mozilla/native-messaging-hosts/`) ou, em
   desenvolvimento, com `scripts/install-native-host.sh` (registro do usuário em
   `~/.mozilla/native-messaging-hosts/`).
2. **Extensão** *Lyra Downloads Integration* (`extensions/firefox`). O RPM não a
   instala: registrar o host não instala extensão nenhuma.

Alvo inicial: Firefox instalado como pacote nativo (MozillaFirefox 140 ESR no
Leap 16). Firefox em **Flatpak ou Snap** não enxerga os manifestos do sistema
nem pode executar o host fora do sandbox sem integração específica (portais);
isso **não foi testado nem é suportado** nesta versão.

## Identificador da extensão

O ID definitivo é `lyra-downloads@lyraos.com.br`, escolhido pelo mantenedor em
2026-09-18 seguindo o padrão das extensões Sheliak. Depois da primeira
assinatura pela Mozilla ele **não pode mudar**. Ele aparece em
`extensions/firefox/static/manifest.json` (`browser_specific_settings.gecko.id`),
em `EXTENSION_ID` (`crates/lyra-downloads-nativehost/src/lib.rs`), no `%build`
do spec (`allowed_extensions` do manifesto do native host) e em
`scripts/install-native-host.sh`; os quatro precisam continuar iguais.

## Desenvolvimento

```sh
cd extensions/firefox
npm ci                 # usa package-lock.json
npm test               # compila o TypeScript e roda os testes de lógica
npm run lint           # web-ext lint
npx web-ext run --source-dir dist   # Firefox com perfil temporário
```

Ou carregar manualmente: `about:debugging#/runtime/this-firefox` →
"Carregar extensão temporária…" → `extensions/firefox/dist/manifest.json`.
Extensões temporárias somem ao fechar o Firefox.

O host precisa estar registrado para o usuário:

```sh
cargo build --workspace
scripts/install-native-host.sh            # aponta para target/…/lyra-downloads-nativehost
scripts/uninstall-native-host.sh          # desfaz
```

## Pacote e assinatura

`npm run package` gera `artifacts/lyra_downloads_integration-<versão>.zip`.
Esse arquivo **não é assinado** e não serve para instalação normal: o Firefox
de lançamento exige extensões assinadas pela Mozilla, e esta integração não
pede para desligar essa verificação.

Fluxo previsto para distribuição (a decidir pelo mantenedor):

- **Listada no AMO** (addons.mozilla.org): publicação pública com revisão.
- **Não listada (self-distribution)**: `web-ext sign --channel=unlisted` com
  credenciais da API do AMO; o XPI assinado é distribuído pelo projeto.

Ambos exigem conta de desenvolvedor Mozilla; nada disso foi feito nesta
entrega. Se um XPI assinado vier a acompanhar o RPM, é preciso conferir o
mecanismo que o Firefox do alvo aceita para extensões instaladas pelo sistema
(por exemplo, diretórios de extensões do sistema ou políticas corporativas) e
documentá-lo antes de usar.

## Uso

- **Envio manual**: botão direito num link → "Baixar com Lyra Downloads". O
  aplicativo abre a janela "Novo download" preenchida; se o usuário cancelar,
  nada é criado.
- **Lyra Downloads como padrão** (desligado por padrão): no botão da extensão
  (ou nas configurações), ligar "Usar o Lyra Downloads para downloads". O
  Firefox pede permissão para acompanhar as requisições dos sites escolhidos
  (todos ou uma lista) e para gerenciar downloads. Desligar devolve essas
  permissões. Continuam sempre no Firefox: janelas privadas, POST,
  autenticação, `blob:`/`data:`, downloads que não podem ser pausados e — se
  não for marcada a opção correspondente — downloads que enviaram cookies.
  O Firefox pode mostrar o início do download por um instante antes do repasse.

Detalhes do protocolo e dos estados de repasse: `docs/PROTOCOL.md`.
