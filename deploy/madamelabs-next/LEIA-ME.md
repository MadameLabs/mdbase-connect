# Stack "mdbase next" no Labs: pacote de instalação

Pacote preparado, não executado. Roda no Labs depois da revisão, pelo Doctor Labs, com o Arthur. Nenhum passo mexe em `/opt/mdbase-connect` nem no `mdbase-connect.service` do `mrlabs`.

## Arquivos

| Arquivo | Para quê |
| --- | --- |
| `compose.yml` | projeto `mdbase-next`: `next-db`, `next-migrate`, `next-server` (`127.0.0.1:8800`), `next-log` (`127.0.0.1:8801`), rede `172.31.249.0/24` |
| `instalar.sh` | instalação root, uma vez; idempotente |
| `desfazer.sh` | derruba a stack e tira só as duas regras de túnel |
| `backup.sh` | `pg_dump` de `mdbase_next` e `mdbase_next_log` e, quando existem, dos opcionais `mdbase_next_hosted` e `mdbase_next_mcp` (desde 2026-11-22), contagem exata por tabela, cifrado com gpg, enviado ao R2 |
| `mdbase-next-backup.service` e `.timer` | backup diário às 21:30 (horário local) |
| `backup-timer.sh` | root, pelo Doctor: instala o `.timer` do backup e, opcionalmente, copia o backup mais recente para o teste de restauração |
| `restaurar-teste.sh` | roda no Arthur-PC, sem root: decifra um backup, restaura num Postgres descartável e confere a contagem por tabela |
| `restaurar-teste.test.sh` | teste do anterior, com backup sintético; não vai ao Labs |
| `liberar-rede.sh` | ficha 2: põe `172.31.249.0/24` no `ISOLADAS` e recarrega, com volta automática |
| `isolada.py` | confere pela tabela carregada que uma faixa está isolada (por cobertura) |
| `log-backend.sh` | troca o backend do `next-log` entre memória e Postgres (só a linha `LOGSVC_BACKEND`) |
| `log-registro.sh` | leva o `next-log` à imagem `.2` com o registro de exclusões em Postgres, com volta automática |
| `atualizar-backup.sh` | root, pelo Doctor: troca só o `backup.sh` instalado (cópia `.antes.<data>`); recusa o de três bancos antes de o `mdbase_next_hosted` existir |
| `mcp-instalar.sh` | root, pelo Doctor: sobe o gateway MCP (`next-mcp`, banco `mdbase_next_mcp`) e publica `mcp-next.madamelabs.com.br` pelo túnel, com volta automática |
| `mcp-registro.sh` | root, pelo Doctor, só leitura: mostra o registro de escritas do gateway (`audit-cli.js`), com os mesmos filtros |
| `ligar-provedor.sh` | root, pelo Doctor: carrega a imagem do provedor hosted, cria o banco `mdbase_next_hosted`, gera o `.env.provedor` e sobe o `next-hosted-provider` na rede do servidor |
| `ligar-convites.sh` | root, pelo Doctor: liga ou desliga as rotas de convite de coleção (`ligar` / `desligar`), com cópia do `.env.servidor` e volta automática |
| `SHA256SUMS` | hashes dos arquivos acima |

## Verificação do pacote (antes de rodar qualquer coisa)

O Doctor confere o pacote com o hash do commit registrado no relatório da Task 5:

```sh
cd /home/mrlabs/mdbase-next && sha256sum -c SHA256SUMS
```

Todos os arquivos têm de dar `OK`. Se algum falhar, parar e avisar o Arthur.

## Segredos (0600, em `/opt/mdbase-next`, separados por serviço)

| Arquivo | Conteúdo | Quem recebe |
| --- | --- | --- |
| `.env.db` | `POSTGRES_PASSWORD` | só `next-db` |
| `.env.servidor` | variáveis do servidor (sem `LOGSVC_*`), `DATABASE_URL`, `MDBASE_CONNECT_GOOGLE_*` e `MDBASE_CONNECT_ALLOWED_GOOGLE_SUBJECTS` | `next-migrate` e `next-server` |
| `.env.log` | `LOGSVC_*` (sem variáveis do servidor) e `LOGSVC_PG_URL` | só `next-log` |

`MDBASE_CONNECT_ALLOWED_GOOGLE_SUBJECTS` é o allowlist de login: é copiado do `.env` do Connect (lá os nomes são curtos, `GOOGLE_CLIENT_ID` e `ALLOWED_GOOGLE_SUBJECTS`, e o `override.yml` dele os repassa com o prefixo), para o novo servidor compartilhar a mesma lista de contas autorizadas. Vazio hoje no Connect até a primeira entrada pelo Google.

`backup.key` (0600) é a chave de cifragem dos backups: ver ficha 5.

## Memória

| Serviço | `mem_limit` |
| --- | --- |
| `next-db` (Postgres 17, `shared_buffers=128MB`, 60 conexões) | 384 MiB |
| `next-server` | 512 MiB |
| `next-log` | 256 MiB |
| `next-migrate` (só na subida) | 256 MiB |

Soma dos três que ficam de pé: 1152 MiB. O Connect atual soma cerca de 2 GiB; o Labs tem cerca de 2,7 GiB livres.

## Ordem de execução (fichas do Doctor)

**Ficha 1, sem root (pela pasta do `mrlabs`):** copiar o conteúdo de `deploy/madamelabs-next/` para `/home/mrlabs/mdbase-next/`, e colocar ali `imagens/` (os dois `.tar.gz` da Task 4, com `.sha256`), `servidor.env` e `log.env` (cópias da `mdbaselabs-segredos/`, modo 600). Rodar a verificação de hash acima.

**Ficha 2, root: liberar a sub-rede no isolamento.** `instalar.sh` recusa rodar se `172.31.249.10` não estiver no set `ISOLADAS` carregado (`ip laboratorio`). O arquivo é `/etc/nftables/laboratorio-rede.nft`, carregado pelo `laboratorio-rede.service` (ver estado-atual-labs, Rede). Pelo Doctor, num passo só, com volta automática:

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/liberar-rede.sh /home/mrlabs/mdbase-next/isolada.py
```

O `isolada.py` vai como argumento porque o Doctor só leva ao preparo (e ao Toxy) os arquivos que aparecem no argv; o script roda de lá, não da pasta do `mrlabs`. À mão, os passos equivalentes:

```sh
sudo cp -p /etc/nftables/laboratorio-rede.nft /etc/nftables/laboratorio-rede.nft.antes-mdbase-next
sudoedit /etc/nftables/laboratorio-rede.nft     # incluir 172.31.249.0/24 em ISOLADAS, no mesmo formato das outras
sudo nft -c -f /etc/nftables/laboratorio-rede.nft   # só checa a sintaxe
sudo systemctl restart laboratorio-rede.service
sudo nft list table ip laboratorio | python3 isolada.py 172.31.249.0/24 && echo isolada   # o nft funde faixas vizinhas ao listar: conferir por cobertura
```

Antes do restart, confirmar com `sudo head -20 /etc/nftables/laboratorio-rede.nft` que o arquivo limpa a tabela antes de recarregar (senão o restart soma regras). Não usar `firewall-cmd --direct`.

**Ficha 3, root: validador do túnel (teste de guarda).** O `instalar.sh` já faz, mas conferir à mão antes da instalação:

```sh
printf 'tunnel: teste\ningress:\n  - hostname: x.exemplo.invalido\n    service: http://127.0.0.1:1\n' > /tmp/config-quebrado.yml
sudo cloudflared tunnel --config /tmp/config-quebrado.yml ingress validate; echo "rc=$?"   # tem de dar rc diferente de 0
sudo rm -f /tmp/config-quebrado.yml
```

Se der `rc=0`, parar: o validador não serve de guarda.

**Ficha 4, root: instalação.** Pelo Doctor, com os arquivos auxiliares como argumentos (o Doctor só leva ao preparo e ao Toxy o que está no argv; sem argumentos o script os lê da própria pasta, para uso manual):

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/instalar.sh /home/mrlabs/mdbase-next/compose.yml /home/mrlabs/mdbase-next/backup.sh /home/mrlabs/mdbase-next/desfazer.sh /home/mrlabs/mdbase-next/mdbase-next-backup.service /home/mrlabs/mdbase-next/mdbase-next-backup.timer /home/mrlabs/mdbase-next/isolada.py
```

Ordem interna:

1. Pré-checagens, sem alterar nada: root; comandos; arquivos; variáveis (por nome) do Google, do servidor e do log; sub-rede no set carregado; sub-rede livre no Docker; 10 GiB livres em `/var/lib/docker`; validador rejeita config quebrado; `connect` e `chat` respondem. **Se o connect ou o chat não responderem, o script para antes de tocar no cloudflared e diz por quê.**
2. sha256 fixado no script das duas imagens e `docker load`.
3. `.env.db`, `.env.servidor`, `.env.log` (0600) e `backup.key` (0600).
4. Certificado TLS do Postgres (autoassinado, SAN `next-db`, 10 anos) no volume `mdbase-next_next-tls`.
5. Sobe `next-db`, cria o banco `mdbase_next_log`, ajusta o dono do volume de objetos, sobe `next-server` (o Compose roda `next-migrate` antes) e `next-log`. Espera `/ready` 200 em `8800` e `/health` 200 em `8801`.
6. **CNAMEs antes do config.yml**, pela API da Cloudflare com o token de DNS do Doctor (`/etc/doctor-labs/cloudflare.ini`), porque o `cloudflared tunnel route dns` precisa do `cert.pem` da conta, que fica com o `mrlabs`. Registro que já aponta para `<tunel>.cfargotunnel.com` passa; com outro destino, o script para e diz qual.
7. **config.yml, só as regras que faltam**, antes do catch-all. Cada regra é conferida com `cloudflared tunnel --config <tmp> ingress validate` e `ingress rule https://<host>` (tem de cair em `127.0.0.1:8800` ou `8801`). Antes de gravar, cópia datada `/etc/cloudflared/config.yml.antes-mdbase-next.<AAAAmmdd-HHMMSS>`. Depois, reinicia o `cloudflared`.
8. Confere `connect.madamelabs.com.br/ready` (200), `chat.madamelabs.com.br` (2xx/3xx), `next.madamelabs.com.br/ready` (200) e `next-log.madamelabs.com.br/health` (200).
9. Ativa o timer `mdbase-next-backup` e apaga `/home/mrlabs/mdbase-next/*.env` com `shred`.

Se falhar depois do passo 7, o trap restaura a cópia **desta execução** (datada), reinicia o `cloudflared` e imprime o estado (`cloudflared` e `connect /ready`). Os contêineres ficam de pé para diagnóstico. Se falhar antes do passo 7, o `config.yml` não foi tocado. As cópias `*.env` ficam em `/home/mrlabs/mdbase-next/` nesse caso: apagar à mão depois do diagnóstico.

**Ficha 5, Arthur, no terminal dele:** guardar `/opt/mdbase-next/backup.key` no Bitwarden (item `mdbase-next · chave de backup`), como o `bin/guardar-chaves` do Connect. Sem essa cópia, um backup não se restaura.

**Ficha 6, Arthur, no painel da Cloudflare:** conferir que a regra do WAF (só Brasil) cobre `next.madamelabs.com.br` e `next-log.madamelabs.com.br`. Hoje ela cita os hosts do Connect e do sync pelo nome.

**Ficha 7, Arthur:** primeiro login em `https://next.madamelabs.com.br` com Google.

## Registro de exclusões em Postgres (depois da instalação)

O backend Postgres do upstream não grava: falta o produtor do piso de exclusão. A imagem `mdbase-next-log:0.1.0-mdbaselabs.2` traz o `PgDeletionRegistry` do fork (commit `e9f7ae72`), ligado por `LOGSVC_PG_DELETION_REGISTRY=1`. O piso fica numa tabela do mesmo Postgres do log (`ls_collection_deletion_floor`): restaurar backup anterior a uma exclusão também esquece o piso.

**Ficha A, sem root:** pôr `mdbase-next-log-0.1.0-mdbaselabs.2.tar.gz` e o `.sha256` em `/home/mrlabs/mdbase-next/imagens/`, atualizar o pacote e rodar `sha256sum -c SHA256SUMS`.

**Ficha B, root, pelo Doctor (a imagem vai como argumento):**

```sh
bash /home/mrlabs/mdbase-next/log-registro.sh /home/mrlabs/mdbase-next/imagens/mdbase-next-log-0.1.0-mdbaselabs.2.tar.gz
```

Confere o sha256 fixado no script, carrega a imagem, copia o `compose.yml` (`.antes-log-registro.<data>`), troca a imagem do log, põe `LOGSVC_BACKEND: pg` e `LOGSVC_PG_DELETION_REGISTRY: "1"` e sobe só o `next-log`. Exige `/ready` e `/health` 200 e a linha `"event":"deletion_registry"` no log; se faltar algo, volta o `compose.yml` e sobe o `next-log` como estava. O conteúdo do log em memória se perde: coleções da prova precisam ser recriadas.

## Backup às 21:30 e teste de restauração

O Labs dorme das ~23h às 6h30, e o backup das 22:50 corria o risco de não rodar. O timer passou para 21:30.

**Ficha C, sem root:** atualizar o pacote em `/home/mrlabs/mdbase-next/` e rodar `sha256sum -c SHA256SUMS`.

**Ficha D, root, pelo Doctor (o timer vai como argumento):**

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/backup-timer.sh /home/mrlabs/mdbase-next/mdbase-next-backup.timer --copiar-para-restauro
```

`backup-timer.sh` guarda o `.timer` instalado (`.antes.<data>`), instala o novo, faz `daemon-reload` e `restart` e mostra `list-timers` e o último sucesso. Esperado: próxima execução às 21:30. Com `--copiar-para-restauro`, copia o backup cifrado mais recente para `/home/mrlabs/restauro` (0600, do `mrlabs`). O Doctor não aceita `bash -c` nem quebra de linha no pedido. Volta: o mesmo comando com a cópia `.antes.<data>`.

**Teste de restauração, no Arthur-PC, sem root.** Precisa de `docker` e `gpg`, do backup mais recente do R2 (bucket `mdbase-connect-backups`, prefixo `mdbase-next/`) e da `backup.key` (Bitwarden, `mdbase-next · chave de backup`), as duas em arquivo 0600 fora do repositório. A nota do Bitwarden tem um título, uma linha em branco e a chave; o gpg lê só a primeira linha do arquivo, então guardar só a linha da chave (64 caracteres base64, `grep -E '^[A-Za-z0-9+/]{64}$'`). Apagar o arquivo da chave (`shred -u`) depois do teste:

```sh
deploy/madamelabs-next/restaurar-teste.sh <mdbase-next-AAAAmmddTHHMMSSZ.tar.gz.gpg> <backup.key>
```

O script decifra, sobe `postgres:17-alpine` (a mesma imagem fixada do `next-db`) com `--network none` e `--rm`, restaura `mdbase_next`, `mdbase_next_log` e, quando o backup o tem, `mdbase_next_hosted` (backups anteriores a 2026-11-22 restauram com aviso), recalcula a contagem com a consulta do `backup.sh` e compara com o `contagem.tsv` do backup. Imprime a contagem por tabela e sai com 0 só se bater. O teste do próprio script: `deploy/madamelabs-next/restaurar-teste.test.sh`.

## Provedor hosted (pré-requisito dos convites)

Design e plano na base: `provedor-hosted-na-stack-next-design` e `provedor-hosted-na-stack-next`. O servidor só liga os convites entre contas com um provedor hosted de verdade. O `next-hosted-provider` não guarda coleção nenhuma: fica na rede do contêiner do servidor (`network_mode: service:next-server`), escuta em `127.0.0.1:8790` e usa o banco `mdbase_next_hosted` do `next-db`, com usuário próprio. Memória medida no Arthur-PC: cerca de 7 MiB ocioso (`mem_limit` de 256 MiB).

**Antes, no painel da Cloudflare (Arthur):** criar o bucket R2 `mdbase-next-hosted` e um token de API R2 com "Object Read & Write" só nesse bucket. O endpoint é o mesmo do Connect (`https://<conta>.r2.cloudflarestorage.com`). No Labs, como `mrlabs`, escrever `/home/mrlabs/mdbase-next/r2-provedor.env` (0600) com exatamente quatro linhas: `MDBASE_CONNECT_R2_ENDPOINT=…`, `MDBASE_CONNECT_R2_BUCKET=mdbase-next-hosted`, `MDBASE_CONNECT_R2_ACCESS_KEY_ID=…` e `MDBASE_CONNECT_R2_SECRET_ACCESS_KEY=…`. Esse arquivo nunca vai no argv, porque o Doctor mostra o argv ao Toxy. O script o lê do caminho fixo, uma vez só, para uma cópia do root, e recusa link simbólico, arquivo não regular e fim de linha CR. No fim ele apaga o arquivo.

**Ficha F, sem root:** atualizar o pacote em `/home/mrlabs/mdbase-next/`, pôr `mdbase-next-hosted-provider-0.1.0-mdbaselabs.1.tar.gz` em `imagens/` e rodar `sha256sum -c SHA256SUMS`.

**Ficha G, root, pelo Doctor:**

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/ligar-provedor.sh /home/mrlabs/mdbase-next/compose.yml
```

Confere o sha256 da imagem e do `compose.yml` e confere que o `compose.yml` em uso é o anterior. Carrega a imagem, cria o papel e o banco, gera o `/opt/mdbase-next/.env.provedor` (0600) se faltar, instala o `compose.yml` (cópia `compose.yml.antes-provedor.<data>`) e sobe só o provedor. Esperado: `ok: provedor pronto em 127.0.0.1:8790 na rede do next-server`. Se o `/ready` não vier, ele tira o provedor e volta o `compose.yml`. O `.env.provedor`, o banco e o `r2-provedor.env` ficam para a próxima tentativa. **Para trocar a credencial do R2** (errada ou renovada): escrever o `r2-provedor.env` de novo e rodar a mesma ficha G. Ela troca só as quatro linhas do R2 no `.env.provedor` (cópia `.antes-r2.<data>`) e recria o provedor. **Volta manual:** `docker compose -p mdbase-next -f /opt/mdbase-next/compose.yml --project-directory /opt/mdbase-next rm -sf next-hosted-provider` e o `compose.yml.antes-provedor.<data>`.

Depois da ficha G, o Arthur guarda no Bitwarden (`mdbase-next · provedor hosted`) a chave mestra e o token interno do `.env.provedor` e as credenciais do R2.

**Ficha H, root, pelo Doctor, só depois da ficha G** (o `backup.sh` novo faz dump do `mdbase_next_hosted`):

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/atualizar-backup.sh /home/mrlabs/mdbase-next/backup.sh
```

Esperado: `ok: backup.sh instalado`. No dia seguinte, conferir o backup das 21:30 com o `restaurar-teste.sh` no Arthur-PC: a saída lista as tabelas do `mdbase_next_hosted`. Volta: o mesmo comando com o `/opt/mdbase-next/backup.sh.antes.<data>`.

## Gateway MCP (plano de migração, Task 6)

`next-mcp` (imagem `mdbase-next-mcp:0.1.0-mdbaselabs.1`, de `deploy/docker/Dockerfile.mcp` na `madamelabs/stack-next`, 4,4 GB carregada) com a migração `next-mcp-migrate`, banco `mdbase_next_mcp` no `next-db` (papel próprio, `sslmode=verify-full` com a CA da stack) e `/opt/mdbase-next/.env.mcp` (0600: `DATABASE_URL` e `MDBASE_MCP_MASTER_KEY`). Porta `127.0.0.1:8802`, publicada pelo túnel em `https://mcp-next.madamelabs.com.br/mcp`. O banco guarda os clientes OAuth e o registro de escritas (`mcp_write_audit`), e o backup o inclui.

**Ficha F de novo, sem root:** atualizar o pacote e pôr `mdbase-next-mcp-0.1.0-mdbaselabs.1.tar.gz` em `imagens/`; `sha256sum -c SHA256SUMS`.

**Ficha I, root, pelo Doctor:**

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/mcp-instalar.sh /home/mrlabs/mdbase-next/compose.yml
```

Esperado: `ok: https://mcp-next.madamelabs.com.br/mcp responde o desafio OAuth; connect e next 200`. Se falhar depois de mexer, volta o `config.yml` do `cloudflared`, tira o `next-mcp` e volta o `compose.yml`. O `.env.mcp` e o banco ficam. Depois, a ficha H de novo (`atualizar-backup.sh`), para o backup das 21:30 incluir o `mdbase_next_mcp`.

**Depois, com o Arthur:** acrescentar `mcp-next.` à regra do WAF "So Brasil - ntfy e next" e guardar a `MDBASE_MCP_MASTER_KEY` no Bitwarden (`mdbase next · mcp master key`), lendo-a no Labs **fora do Claude** (`ssh -t labs sudo less /opt/mdbase-next/.env.mcp`).

## Convites entre contas (prova da migração, Task 4)

Uma coleção Private nativa ganha membro de outra conta pelo convite (`POST /v1/hosted/collections/:id/invitations`, pelo portal ou pela API). O servidor só monta essas rotas com `MDBASE_CONNECT_HOSTED_COLLECTIONS=1`, `MDBASE_CONNECT_HOSTED_SHARING_ENABLED=1` e um provedor hosted que responda ao `/ready`: o `next-hosted-provider` da seção anterior, que precisa estar de pé (ficha G) antes desta. O script aponta o servidor para `http://127.0.0.1:8790` com o token interno do `.env.provedor`. Recriar o servidor tira a rede do provedor, então ele recria os dois juntos.

**Ficha C, sem root:** atualizar o pacote em `/home/mrlabs/mdbase-next/` e rodar `sha256sum -c SHA256SUMS`.

**Ficha E, root, pelo Doctor:**

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/ligar-convites.sh ligar
```

Guarda o `.env.servidor` em `.env.servidor.antes-convites`, acrescenta as quatro linhas, recria o `next-server` e o `next-hosted-provider` e espera o `/ready` dos dois. Se o servidor não voltar, restaura a cópia sozinho. Esperado no fim: `GET …/invitations=401` (rota montada, pede login) e `ok: convites montados`. **Volta**, depois da prova ou se algo der errado:

```sh
doctor-pedir -- bash /home/mrlabs/mdbase-next/ligar-convites.sh desligar
```

## Desfazer

`sudo /opt/mdbase-next/desfazer.sh`: derruba o projeto (volumes ficam, `.env.*` e `backup.key` ficam), tira só as duas regras do `config.yml` (o resto, inclusive mudanças posteriores à instalação, fica), com cópia datada antes de gravar. Confere `cloudflared` e `connect /ready`; se algo falhar, restaura a cópia datada e imprime o estado. Os CNAMEs saem pelo painel da Cloudflare (o `cloudflared` não tem comando para apagar CNAME). A isolamento `172.31.249.0/24` no nftables não é desfeito pelo script: tirar à mão, com o backup `.antes-mdbase-next`, se for preciso.

## Portal e Google

- O portal é servido pelo próprio `next-server`: a imagem traz `PORTAL_DIST=/app/apps/portal/dist`, registrado na raiz. Origem: `https://next.madamelabs.com.br` (`PUBLIC_URL`).
- O login Google usa Google Identity Services no portal, validado pela origem da página. A origem **`https://next.madamelabs.com.br`** já está nas origens JavaScript do cliente OAuth. **Origem extra necessária: não.**
- O editor (`apps/editor`) não está na imagem nem neste pacote. Se for publicado em outra origem com login Google, essa origem entra no cliente OAuth. Nenhuma foi definida.

## Riscos abertos

- **Sub-rede `172.31.249.0/24`:** `254` e `255` são do `evolution_internal` e do `navegador_internal`; `250`–`253` em uso. A `249` não foi verificada no Labs (sem acesso); o script confere antes de criar a rede.
- **`next-server` com `read_only`:** igual ao Connect, nunca rodou assim neste servidor. Se o `/ready` falhar por escrita, o log do contêiner mostra o caminho.
- **`next-log` chama o servidor pela URL pública** (`https://next-log...`), porque o código exige https fora de loopback. Sai pelo Cloudflare e volta ao host.
- **TLS do Postgres:** autoassinado, 10 anos. Renovação: refazer o volume `next-tls` e reiniciar os três serviços.
- **`MDBASE_CONNECT_IDENTITY_ISSUER`:** igual à `PUBLIC_URL`; confirmar com o Arthur.
- **Registro:** `MDBASE_CONNECT_REGISTRATION=closed` fixo no compose. Conferir antes do primeiro login se o `ALLOWED_GOOGLE_SUBJECTS` copiado do Connect basta.
- **Backup:** o molde `/opt/mdbase-connect/bin/backup` é root e não foi lido. `backup.sh` segue a descrição do `estado-atual-labs.md`. Conferir bucket (`mdbase-connect-backups`, prefixo `mdbase-next/`), endpoint e variáveis `MDBASE_CONNECT_R2_*`. Retenção de 30 dias no R2 não está no script (regra de ciclo de vida no bucket). Restauração: testar com `restaurar-teste.sh` (seção abaixo).
- **Política de confiança expira em 2027-11-17** (`servidor.env`); o servidor recusa com menos de 7 dias.
- **Formato de saída do `cloudflared ingress rule`:** o script procura `http://127.0.0.1:<porta>` na saída. Conferir no primeiro uso.
