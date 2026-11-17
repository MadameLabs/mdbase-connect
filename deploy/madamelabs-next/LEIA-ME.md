# Pilha "mdbase next" no Labs: pacote de instalação

Pacote preparado, não executado. Roda no Labs depois da revisão, pelo Doctor Labs, com o Arthur. Nenhum passo deste pacote mexe em `/opt/mdbase-connect` nem no `mdbase-connect.service` do `mrlabs`.

## Arquivos

| Arquivo | Para quê |
| --- | --- |
| `compose.yml` | projeto `mdbase-next`: `next-db`, `next-migrate`, `next-server` (porta `127.0.0.1:8800`), `next-log` (porta `127.0.0.1:8801`), rede `172.31.249.0/24` |
| `instalar.sh` | instalação root, uma vez; idempotente |
| `backup.sh` | `pg_dump` dos dois bancos, cifrado com gpg, enviado ao R2 |
| `mdbase-next-backup.service` e `.timer` | backup diário às 22:50 (horário local) |
| `desfazer.sh` | derruba a pilha e tira as duas regras de túnel |

## Memória

| Serviço | `mem_limit` |
| --- | --- |
| `next-db` (Postgres 17, `shared_buffers=128MB`, 60 conexões) | 384 MiB |
| `next-server` | 512 MiB |
| `next-log` | 256 MiB |
| `next-migrate` (só durante a subida, depois sai) | 256 MiB |

Soma dos três que ficam de pé: 1152 MiB (cerca de 1,1 GiB). O Connect atual soma cerca de 2 GiB e o Labs tem cerca de 2,7 GiB livres.

## Ordem de execução

1. **Ficha 1, sem root (Arthur ou Doctor, pela pasta do `mrlabs`):** copiar a pasta `deploy/madamelabs-next/` para `/home/mrlabs/mdbase-next/`. Ali precisam estar: `imagens/` (os dois `.tar.gz` do Task 4), `servidor.env` e `log.env` (cópias da `mdbaselabs-segredos/`, modo 600). O `.env` do Connect (`/opt/mdbase-connect/.env`) só é lido pelo nome das variáveis.
2. **Ficha 2, root: liberar a sub-rede no isolamento.** `instalar.sh` recusa rodar se `172.31.249.0/24` não estiver em `ISOLADAS` de `/etc/nftables/laboratorio-rede.nft`. Fazer backup do arquivo (`.antes-mdbase-next`), incluir a sub-rede no mesmo formato das outras, recarregar a tabela `ip laboratorio` e conferir que ela sobrevive ao `firewall-cmd --reload`. Não usar `firewall-cmd --direct`.
3. **Ficha 3, root: instalação.** `sudo bash /home/mrlabs/mdbase-next/instalar.sh`. Ordem interna:
   1. pré-checagens (root, comandos, arquivos, variáveis do Google por nome, sub-rede livre, isolamento). Só leitura.
   2. confere o sha256 fixado no script das duas imagens e faz `docker load`.
   3. cria `/opt/mdbase-next` (0711) e o `.env` (0600) com `servidor.env`, `log.env`, as variáveis `MDBASE_CONNECT_GOOGLE_*` e `MDBASE_CONNECT_ALLOWED_GOOGLE_SUBJECTS` do Connect, e uma senha nova do Postgres. Gera `backup.key`.
   4. gera o certificado TLS do Postgres (autoassinado, SAN `next-db`, 10 anos) no volume `mdbase-next_next-tls`.
   5. sobe `next-db`, cria o banco `mdbase_next_log`, ajusta o dono do volume de objetos do log, sobe `next-server` (o Compose roda `next-migrate` antes) e `next-log`. Espera `/ready` 200 em `8800` e `/health` 200 em `8801`.
   6. faz backup de `/etc/cloudflared/config.yml` (`.antes-mdbase-next`), insere as regras `next.madamelabs.com.br` e `next-log.madamelabs.com.br` antes do catch-all, valida com `cloudflared --config <tmp> tunnel ingress validate`, cria os CNAMEs com `cloudflared tunnel route dns <tunel> <host>`, reinicia o `cloudflared`.
   7. confere `https://connect.madamelabs.com.br/ready` (200), `https://chat.madamelabs.com.br/` (2xx ou 3xx), `https://next.madamelabs.com.br/ready` (200) e `https://next-log.madamelabs.com.br/health` (200).
   8. ativa o timer `mdbase-next-backup`, apaga `/home/mrlabs/mdbase-next/*.env` (com `shred`).
   
   Se qualquer passo falhar depois do passo 6, o trap restaura o `config.yml` do backup e reinicia o `cloudflared`. Os contêineres ficam de pé para diagnóstico; `desfazer.sh` derruba. Se falhar antes disso, o `config.yml` não foi tocado. As cópias `*.env` ficam em `/home/mrlabs/mdbase-next/` nesse caso; apagar à mão depois de diagnosticar.
4. **Ficha 4, Arthur, no painel da Cloudflare:** conferir que a regra do WAF que bloqueia fora do Brasil cobre `next.madamelabs.com.br` e `next-log.madamelabs.com.br`. Hoje a regra cita os hosts do Connect e do sync pelo nome.
5. **Ficha 5, Arthur, no terminal dele:** guardar `/opt/mdbase-next/backup.key` no Bitwarden (item `mdbase-next · chave de backup`), com o mesmo procedimento de `bin/guardar-chaves` do Connect. Sem essa cópia, um backup não se restaura.
6. **Ficha 6, Arthur:** primeiro login em `https://next.madamelabs.com.br` com Google. Ver "Registro e acesso" abaixo.

## Desfazer

`sudo /opt/mdbase-next/desfazer.sh`: derruba o projeto (volumes ficam), tira só as duas regras do `config.yml` (o resto do arquivo, inclusive mudanças posteriores à instalação, fica), reinicia o `cloudflared` e lembra os dois CNAMEs. O `cloudflared` não tem comando para apagar CNAME: sair pelo painel da Cloudflare.

## Portal e Google

- O portal é servido pelo próprio `next-server`: a imagem traz `PORTAL_DIST=/app/apps/portal/dist` e o servidor registra esse diretório com `fastify-static` na raiz. A origem é `https://next.madamelabs.com.br` (`PUBLIC_URL`).
- O login do Google usa Google Identity Services no portal (`apps/portal/src/GoogleIdentityButton.tsx`), que valida a origem da página. Origem necessária: **`https://next.madamelabs.com.br`**, já salva pelo Arthur nas origens JavaScript do cliente OAuth. Não é preciso outra origem para o portal.
- O editor (`apps/editor`) não está na imagem nem neste pacote. Se for publicado em outra origem e tiver login do Google, essa origem precisa entrar no cliente OAuth. Nenhuma foi definida ainda: **extra origin needed: não**, para esta instalação.

## Registro e acesso

- `MDBASE_CONNECT_REGISTRATION` sai de `NEXT_REGISTRATION_MODE`, com padrão `closed`. Antes do primeiro login do Arthur, decidir se o `ALLOWED_GOOGLE_SUBJECTS` vai ficar vazio (como no Connect) ou com o sub do Arthur, e se o modo fica `closed` ou `invite`. Ver `IDENTITY_ISSUER` em "Riscos".
- Nenhum `MDBASE_CONNECT_DEV_AUTH`, `MDBASE_NEXT_PRIVATE_BOOTSTRAP` ou `MDBASE_CONNECT_ALLOW_INSECURE_MANIFESTS`.

## Riscos abertos

- **Sub-rede `172.31.249.0/24`:** escolhida porque `254` e `255` são do `evolution_internal` e do `navegador_internal`, e `250`–`253` estão em uso. O brief dizia que `254` estava livre; não está. A sub-rede `249` não foi verificada no Labs (sem acesso). O `instalar.sh` confere com `docker network inspect` antes de criar.
- **`next-server` com `read_only`:** igual ao Connect, mas o servidor da pilha nova nunca rodou em `read_only` no Labs. Se o primeiro `/ready` falhar por escrita, o log do contêiner mostra o caminho.
- **`next-log` chama o servidor pela URL pública:** o `MDBASE_NEXT_LOG_SERVICE_URL` é `https://next-log.madamelabs.com.br` porque o código exige https fora de loopback. O tráfego sai do contêiner, passa pelo Cloudflare e volta ao host. Não há caminho interno.
- **TLS do Postgres:** o certificado é autoassinado (10 anos). O `NODE_EXTRA_CA_CERTS` e `LOGSVC_PG_CA_FILE` apontam para ele. Renovação: refazer o volume `next-tls` e reiniciar os três serviços.
- **`MDBASE_CONNECT_IDENTITY_ISSUER`:** o código exige origem exata e https. Está igual à `PUBLIC_URL`. Conferir com o Arthur se o emissor da pilha nova deve ser ele mesmo.
- **Backup:** o molde `/opt/mdbase-connect/bin/backup` é root e não foi lido ao escrever este pacote. `backup.sh` segue a descrição do `estado-atual-labs.md`. **Conferir** o nome do bucket (`mdbase-connect-backups` por padrão, prefixo `mdbase-next/`), o endpoint e as variáveis `MDBASE_CONNECT_R2_*`. Retenção de 30 dias no R2 não está no script: configurar regra de ciclo de vida no bucket.
- **Restauração não testada:** `backup.sh` não tem `testar-restauracao`. Antes de confiar no backup, restaurar num Postgres descartável.
- **Certificado do Postgres e chave de política:** a política expira em 2027-11-17 (`servidor.env`); o servidor recusa com menos de 7 dias.
- **Imagem do servidor de 4,47 GB:** conferir o disco antes do `docker load` (no Labs havia 67 GB livres em 2026-11-14).
