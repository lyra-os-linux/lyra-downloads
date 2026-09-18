# Arquitetura do Lyra Downloads

## Componentes

```
 Firefox ──(Native Messaging)──▶ lyra-downloads-nativehost ─┐
                                                             │  socket Unix
 lyra-downloads (GTK4/libadwaita) ───────────────────────────┼──────────────▶ lyra-downloads-backend ──JSON-RPC──▶ aria2c
                                                             │   (JSON por linha)      │ (dono da fila)       (loopback,
                                                                                       ▼                       segredo)
                                                                                SQLite + sessão aria2
```

| Crate / diretório | Papel |
|---|---|
| `crates/lyra-downloads-core` | Domínio (`Task`, estados, perfis de conexão), preferências, diretórios XDG, sanitização de nomes, SQLite. Síncrono, sem rede. |
| `crates/lyra-downloads-aria2` | Cliente JSON-RPC do aria2 e supervisão do processo `aria2c` exclusivo. Tradução de códigos de erro. |
| `crates/lyra-downloads-ipc` | Protocolo local (tipos de requisição/resposta) e cliente síncrono do backend, incluindo o início desanexado do backend. |
| `crates/lyra-downloads-backend` | Binário `lyra-downloads-backend`: dono único da fila e do motor, servidor do socket, reconciliação, verificação SHA-256, notificações. |
| `crates/lyra-downloads-gtk` | Binário `lyra-downloads`: interface. Só apresenta e envia comandos ao backend. |
| `crates/lyra-downloads-nativehost` | Binário `lyra-downloads-nativehost`: enquadramento Native Messaging e encaminhamento de operações permitidas. |
| `tools/testserver` | Servidor HTTP local para testes (ranges/206, redirecionamento, falhas). Não é empacotado. |

## Ciclo de vida dos processos

- **Backend**: instância única por usuário, garantida por `flock` em
  `$XDG_RUNTIME_DIR/lyra-downloads/backend.lock`. É iniciado sob demanda pela
  interface ou pelo native host, sempre com `setsid` (nova sessão, stdio em
  `/dev/null`), portanto não pertence ao grupo de processos de quem o iniciou.
  Se o Firefox encerrar o grupo do native host, o backend e o aria2 continuam.
- **Janela**: fechar a janela encerra só a interface. Os downloads seguem.
  "Pausar tudo e encerrar o serviço" (menu principal) pausa tudo, salva a
  sessão e encerra backend e aria2.
- **aria2c**: filho do backend, com porta aleatória em `127.0.0.1`, segredo
  RPC aleatório gravado num arquivo de configuração `0600` (não aparece em
  `ps`), sessão e log próprios em `$XDG_DATA_HOME/lyra-downloads/aria2/`.
  Nunca reutiliza nem sinaliza instâncias de aria2 de terceiros. Se um backend
  anterior caiu e deixou o seu aria2c vivo, o novo backend só o encerra depois
  de confirmar em `/proc/<pid>` que é um `aria2c` com o **nosso** `--conf-path`.
- **Reinícios do motor**: no máximo 3 em 10 minutos; depois disso a interface
  mostra o problema e oferece "Tentar iniciar o motor". Não há laço infinito.

## Autoridade dos dados

| Fonte | Guarda | Decide |
|---|---|---|
| SQLite (`$XDG_DATA_HOME/lyra-downloads/lyra-downloads.db`) | identidade (`Task.id`), URL, destino, perfil de conexões, hash esperado, intenção (pausado **pelo usuário** ou pelo sistema), histórico e resultado | quais tarefas existem |
| aria2 (RPC) | estado operacional: ativo/aguardando/pausado/erro/completo, bytes, velocidade, conexões | o estado momentâneo |
| Sessão do aria2 + arquivos `.aria2` | dados binários de retomada | como continuar do ponto parcial |

- O motor só recebe tarefas que já estão gravadas no banco. Cada tarefa pede
  ao aria2 um **GID determinístico** (16 hex derivados do `Task.id`). Se o
  aria2 recusar (GID em uso no processo), aceita-se o GID atribuído e o
  mapeamento é atualizado no banco.
- Na inicialização (e após reiniciar o motor) o aria2 sobe com
  `--pause=true`: tudo o que ele restaura da sessão começa pausado e o backend
  decide o que retomar, conforme a preferência "Retomar automaticamente" e a
  intenção do usuário (pausas do usuário são sempre respeitadas).
- Uma tarefa ativa no banco que o motor não conhece é reenviada com o mesmo
  GID, destino e nome; o aria2 retoma pelo arquivo `.aria2`. Nunca se cria
  uma segunda tarefa.
- O progresso é persistido a cada 5 s ou em mudanças de estado (não a cada tick).

## Estados

`aguardando → baixando → (verificando) → concluído`, com `pausado`,
`cancelado` e `erro` como desvios. `concluído` carrega também o resultado da
verificação: **SHA-256 confere**, **SHA-256 não confere** (arquivo mantido)
ou **não verificado**. Conferir o hash só prova que os bytes correspondem ao
valor informado; a confiança depende da origem desse valor.

Três ações distintas: **cancelar** (para a transferência, mantém parciais),
**remover do histórico** (apaga só metadados) e **excluir arquivo** (ação
explícita, com confirmação; apaga arquivo e `.aria2` e remove da lista).

## Erros e novo link

Erros do aria2 são traduzidos a partir do código oficial (tabela "Exit status"
do manual, conferida na 1.37.0) e, quando o aria2 informa, do status HTTP.
Um 403 é descrito com causas possíveis, sem afirmar expiração. "Tentar
novamente" reenvia o mesmo link (o aria2 tem limite de 5 tentativas; o
backend não repete sozinho). "Tentar com um novo link" cria **outra tarefa**
com nome sem colisão: parciais de uma URL diferente nunca são reaproveitados.

## Segurança local

- Socket `0600` em diretório `0700`; o backend recusa conexões de outro UID
  (`SO_PEERCRED`).
- Somente operações da lista fechada do protocolo são aceitas; nada de RPC
  arbitrário chega ao aria2. O native host aceita um conjunto ainda menor.
- URLs completas ficam só no banco privado; logs usam `redact_url` (sem
  credenciais, query string ou fragmento).
- Processos são iniciados com argumentos separados, nunca via shell.
- TLS sempre validado (`--check-certificate=true`; a consulta de nome usa os
  certificados do sistema).
- Nomes de arquivo são sanitizados (sem separadores, `..` ou controle) e o
  destino precisa ser absoluto, existente e gravável; colisões recebem
  sufixo ` (1)`, ` (2)`… considerando também `.aria2` e tarefas pendentes.
