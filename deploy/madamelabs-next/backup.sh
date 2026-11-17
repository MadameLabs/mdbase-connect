#!/usr/bin/env bash
# backup.sh: pg_dump dos dois bancos da pilha mdbase next, cifrado com gpg (AES-256) e enviado ao R2.
# CONFERIR antes de confiar: foi escrito a partir da descrição em estado-atual-labs.md (item 8),
# porque o molde /opt/mdbase-connect/bin/backup é root e não pôde ser lido ao escrever este arquivo.
# Credenciais do R2 são lidas por nome do .env do Connect atual; nenhum valor é impresso.
set -euo pipefail
umask 077

DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CONNECT_ENV=/opt/mdbase-connect/.env
R2_BUCKET=${MDBASE_NEXT_BACKUP_BUCKET:-mdbase-connect-backups}
R2_PREFIXO=mdbase-next
LOCAL=/var/backups/mdbase-next
CHAVE="$DESTINO/backup.key"

le_env() { grep -E "^$1=" "$CONNECT_ENV" | tail -n1 | cut -d= -f2- | sed -e "s/^['\"]//" -e "s/['\"]\$//"; }

R2_ID=$(le_env MDBASE_CONNECT_R2_ACCESS_KEY_ID)
R2_SEGREDO=$(le_env MDBASE_CONNECT_R2_SECRET_ACCESS_KEY)
R2_ENDPOINT=$(le_env MDBASE_CONNECT_R2_ENDPOINT)
[[ -n $R2_ID && -n $R2_SEGREDO && -n $R2_ENDPOINT ]] || { echo "credenciais do R2 ausentes em $CONNECT_ENV" >&2; exit 1; }
[[ -f $CHAVE ]] || { echo "chave de cifragem ausente: $CHAVE" >&2; exit 1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$LOCAL"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
nome="mdbase-next-$stamp.tar.gz.gpg"

cd "$DESTINO"
dc() { docker compose -p "$PROJETO" -f compose.yml --project-directory "$DESTINO" "$@"; }

dc exec -T next-db pg_dump -Fc -U mdbase_next -d mdbase_next > "$TMP/mdbase_next.dump"
dc exec -T next-db pg_dump -Fc -U mdbase_next -d mdbase_next_log > "$TMP/mdbase_next_log.dump"
{
  for banco in mdbase_next mdbase_next_log; do
    tabelas=$(dc exec -T next-db psql -U mdbase_next -d "$banco" -tAc "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")
    echo "$banco tabelas=$tabelas"
  done
} > "$TMP/contagem.txt"

tar -C "$TMP" -czf - mdbase_next.dump mdbase_next_log.dump contagem.txt \
  | gpg --batch --yes --pinentry-mode loopback --passphrase-file "$CHAVE" \
        --symmetric --cipher-algo AES256 --output "$LOCAL/$nome"

# Envio ao R2 com SigV4; a credencial vai por arquivo de configuração do curl, não na linha de comando.
printf 'user = "%s:%s"\n' "$R2_ID" "$R2_SEGREDO" > "$TMP/curl.cfg"
curl -fsS --aws-sigv4 "aws:amz:auto:s3" -K "$TMP/curl.cfg" \
  -T "$LOCAL/$nome" -o /dev/null \
  "$R2_ENDPOINT/$R2_BUCKET/$R2_PREFIXO/$nome"

date -u +%Y-%m-%dT%H:%M:%SZ > "$LOCAL/.ultimo-sucesso"
find "$LOCAL" -type f -name 'mdbase-next-*.tar.gz.gpg' -mtime +7 -delete
echo "backup $nome enviado a $R2_BUCKET/$R2_PREFIXO"
