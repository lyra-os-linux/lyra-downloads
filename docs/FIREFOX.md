# Integração com o Firefox

A integração tem duas partes independentes:

1. **Native host** (`lyra-downloads-nativehost`) + registro
   `org.lyraos.downloads.json`. Vem no RPM `lyra-downloads-firefox-integration`
   (registro global em `/usr/lib64/mozilla/native-messaging-hosts/`) ou, em
   desenvolvimento, com `scripts/install-native-host.sh` (registro do usuário em
   `~/.mozilla/native-messaging-hosts/`).
2. **Extensão** *Lyra Downloads Integration*, no repositório
   [lyra-firefox-ext](https://github.com/lyra-os-linux/lyra-firefox-ext). É
   assinada pela Mozilla (AMO, não listada) e vem no RPM `lyra-firefox-ext`
   (`/usr/share/lyra-firefox-ext/`); no Lyra OS o Firefox a instala pela
   política `ExtensionSettings` da imagem. Registrar o host não instala
   extensão nenhuma.

Alvo inicial: Firefox instalado como pacote nativo (MozillaFirefox 140 ESR no
Leap 16). Firefox em **Flatpak ou Snap** não enxerga os manifestos do sistema
nem pode executar o host fora do sandbox sem integração específica (portais);
isso **não foi testado nem é suportado** nesta versão.

## Identificador da extensão

O ID definitivo é `lyra-downloads@lyraos.com.br`, escolhido pelo mantenedor em
2026-09-18 seguindo o padrão das extensões Sheliak. Depois da primeira
assinatura pela Mozilla ele **não pode mudar**. Ele aparece
no `static/manifest.json` do lyra-firefox-ext e, aqui, em `EXTENSION_ID`
(`crates/lyra-downloads-nativehost/src/lib.rs`), no `%build` do spec
(`allowed_extensions` do manifesto do native host) e em
`scripts/install-native-host.sh`; todos precisam continuar iguais.

## Desenvolvimento e assinatura

Desenvolvimento, testes, assinatura e empacotamento da extensão estão no
README do [lyra-firefox-ext](https://github.com/lyra-os-linux/lyra-firefox-ext).
Para testar com um build local do host:

```sh
cargo build --workspace
scripts/install-native-host.sh            # aponta para target/…/lyra-downloads-nativehost
scripts/uninstall-native-host.sh          # desfaz
```

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
