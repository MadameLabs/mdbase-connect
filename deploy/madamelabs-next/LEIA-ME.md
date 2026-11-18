# Pilha "mdbase next" no Labs: pacote de instalação

Pacote preparado, não executado. Roda no Labs depois da revisão, pelo Doctor Labs, com o Arthur. Nenhum passo mexe em `/opt/mdbase-connect` nem no `mdbase-connect.service` do `mrlabs`.

## Arquivos

| Arquivo | Para quê |
| --- | --- |
| `compose.yml` | projeto `mdbase-next`: `next-db`, `next-migrate`, `next-server` (`127.0.0.1:8800`), `next-log` (`127.0.0.1:8801`), rede `172.31.249.0/24` |
| `instalar.sh` | instalação root, uma vez; idempotente |
| `desfazer.sh` | derruba a pilha e tira só as duas regras de túnel |
| `backup.sh` | `pg_dump` dos dois bancos, contagem exata por tabela, cifrado com gpg, enviado ao R2 |
| `mdbase-next-backup.service` e `.timer` | backup diário às 22:50 (horário local) |
| `liberar-rede.sh` | ficha 2: põe `172.31.249.0/24` no `ISOLADAS` e recarrega, com volta automática |
| `isolada.py` | confere pela tabela carregada que uma faixa está isolada (por cobertura) |
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
6. **CNAMEs antes do config.yml.** Para cada host sem regra: `cloudflared tunnel route dns <tunel> <host>`. Se o registro já existir, o script para e pede conferência manual: no painel da Cloudflare, o CNAME tem de apontar para `<tunel>.cfargotunnel.com`. Confirmado, rodar de novo com `doctor-pedir -- env CNAME_CONFERIDO=1 bash /home/mrlabs/mdbase-next/instalar.sh <os mesmos argumentos>`. (`sudo VAR=valor` não serve: o sudo descarta variáveis fora da lista permitida sem SETENV; `env` é o comando e passa a variável.)
7. **config.yml, só as regras que faltam**, antes do catch-all. Cada regra é conferida com `cloudflared tunnel --config <tmp> ingress validate` e `ingress rule https://<host>` (tem de cair em `127.0.0.1:8800` ou `8801`). Antes de gravar, cópia datada `/etc/cloudflared/config.yml.antes-mdbase-next.<AAAAmmdd-HHMMSS>`. Depois, reinicia o `cloudflared`.
8. Confere `connect.madamelabs.com.br/ready` (200), `chat.madamelabs.com.br` (2xx/3xx), `next.madamelabs.com.br/ready` (200) e `next-log.madamelabs.com.br/health` (200).
9. Ativa o timer `mdbase-next-backup` e apaga `/home/mrlabs/mdbase-next/*.env` com `shred`.

Se falhar depois do passo 7, o trap restaura a cópia **desta execução** (datada), reinicia o `cloudflared` e imprime o estado (`cloudflared` e `connect /ready`). Os contêineres ficam de pé para diagnóstico. Se falhar antes do passo 7, o `config.yml` não foi tocado. As cópias `*.env` ficam em `/home/mrlabs/mdbase-next/` nesse caso: apagar à mão depois do diagnóstico.

**Ficha 5, Arthur, no terminal dele:** guardar `/opt/mdbase-next/backup.key` no Bitwarden (item `mdbase-next · chave de backup`), como o `bin/guardar-chaves` do Connect. Sem essa cópia, um backup não se restaura.

**Ficha 6, Arthur, no painel da Cloudflare:** conferir que a regra do WAF (só Brasil) cobre `next.madamelabs.com.br` e `next-log.madamelabs.com.br`. Hoje ela cita os hosts do Connect e do sync pelo nome.

**Ficha 7, Arthur:** primeiro login em `https://next.madamelabs.com.br` com Google.

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
- **Backup:** o molde `/opt/mdbase-connect/bin/backup` é root e não foi lido. `backup.sh` segue a descrição do `estado-atual-labs.md`. Conferir bucket (`mdbase-connect-backups`, prefixo `mdbase-next/`), endpoint e variáveis `MDBASE_CONNECT_R2_*`. Retenção de 30 dias no R2 não está no script (regra de ciclo de vida no bucket). Restauração não testada: restaurar num Postgres descartável antes de confiar.
- **Política de confiança expira em 2027-11-17** (`servidor.env`); o servidor recusa com menos de 7 dias.
- **Formato de saída do `cloudflared ingress rule`:** o script procura `http://127.0.0.1:<porta>` na saída. Conferir no primeiro uso.
