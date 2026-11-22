#!/usr/bin/env bash
# backup.sh: pg_dump dos dois bancos da stack mdbase next, contagem exata por tabela,
# cifrado com gpg (AES-256) e enviado ao R2.
# CONFERIR antes de confiar: escrito a partir da descrição em estado-atual-labs.md (item 8); o molde
# /opt/mdbase-connect/bin/backup é root e não pôde ser lido. Credenciais lidas por nome do .env do Connect; nenhum valor é impresso.
set -euo pipefail
umask 077

DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CONNECT_ENV=/opt/mdbase-connect/.env
R2_BUCKET=${MDBASE_NEXT_BACKUP_BUCKET:-mdbase-connect-backups}
R2_PREFIXO=mdbase-next
LOCAL=/var/backups/mdbase-next
CHAVE="$DESTINO/backup.key"

falha() { echo "backup: FALHA: $*" >&2; exit 1; }
le_env() { grep -E "^$1=" "$CONNECT_ENV" | tail -n1 | cut -d= -f2- | sed -e "s/^['\"]//" -e "s/['\"]\$//"; }

[[ -f $CONNECT_ENV ]] || falha "$CONNECT_ENV não existe"
R2_ID=$(le_env MDBASE_CONNECT_R2_ACCESS_KEY_ID)
[[ -n $R2_ID ]] || falha "variável MDBASE_CONNECT_R2_ACCESS_KEY_ID ausente em $CONNECT_ENV"
R2_SEGREDO=$(le_env MDBASE_CONNECT_R2_SECRET_ACCESS_KEY)
[[ -n $R2_SEGREDO ]] || falha "variável MDBASE_CONNECT_R2_SECRET_ACCESS_KEY ausente em $CONNECT_ENV"
R2_ENDPOINT=$(le_env MDBASE_CONNECT_R2_ENDPOINT)
[[ -n $R2_ENDPOINT ]] || falha "variável MDBASE_CONNECT_R2_ENDPOINT ausente em $CONNECT_ENV"
[[ -f $CHAVE ]] || falha "chave de cifragem ausente: $CHAVE"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$LOCAL"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
nome="mdbase-next-$stamp.tar.gz.gpg"

cd "$DESTINO"
dc() { docker compose -p "$PROJETO" -f compose.yml --project-directory "$DESTINO" "$@"; }

dc exec -T next-db pg_dump -Fc -U mdbase_next -d mdbase_next > "$TMP/mdbase_next.dump"
dc exec -T next-db pg_dump -Fc -U mdbase_next -d mdbase_next_log > "$TMP/mdbase_next_log.dump"
# Bancos opcionais (provedor hosted, gateway MCP): só entram se existirem, para a falta de um
# não derrubar o backup dos principais.
BANCOS=(mdbase_next mdbase_next_log)
for opcional in mdbase_next_hosted mdbase_next_mcp; do
  if [ "$(dc exec -T next-db psql -U mdbase_next -d mdbase_next -tAc "SELECT 1 FROM pg_database WHERE datname='$opcional'")" = 1 ]; then
    dc exec -T next-db pg_dump -Fc -U mdbase_next -d "$opcional" > "$TMP/$opcional.dump"
    BANCOS+=("$opcional")
  else
    echo "aviso: banco $opcional ausente; backup sem ele" >&2
  fi
done

# Contagem exata por tabela (SELECT count(*) real, via query_to_xml), como no backup do Connect.
CONTAGEM_SQL="SELECT table_name, (xpath('/row/c/text()', query_to_xml(format('select count(*) as c from %I.%I', table_schema, table_name), false, true, '')))[1]::text AS linhas FROM information_schema.tables WHERE table_schema='public' AND table_type='BASE TABLE' ORDER BY 1"
{
  for banco in "${BANCOS[@]}"; do
    echo "== $banco"
    dc exec -T next-db psql -v ON_ERROR_STOP=1 -U mdbase_next -d "$banco" -tAF $'\t' -c "$CONTAGEM_SQL"
  done
} > "$TMP/contagem.tsv"

tar -C "$TMP" -czf - $(printf '%s.dump ' "${BANCOS[@]}") contagem.tsv \
  | gpg --batch --yes --pinentry-mode loopback --passphrase-file "$CHAVE" \
        --symmetric --cipher-algo AES256 --output "$LOCAL/$nome"

# Envio ao R2 com SigV4; a credencial vai por arquivo de configuração do curl, não na linha de comando.
printf 'user = "%s:%s"\n' "$R2_ID" "$R2_SEGREDO" > "$TMP/curl.cfg"
curl -fsS --aws-sigv4 "aws:amz:auto:s3" -K "$TMP/curl.cfg" \
  -T "$LOCAL/$nome" -o /dev/null \
  "$R2_ENDPOINT/$R2_BUCKET/$R2_PREFIXO/$nome" || falha "envio ao R2 falhou (o arquivo local fica em $LOCAL/$nome)"

date -u +%Y-%m-%dT%H:%M:%SZ > "$LOCAL/.ultimo-sucesso"
find "$LOCAL" -type f -name 'mdbase-next-*.tar.gz.gpg' -mtime +7 -delete
echo "backup $nome enviado a $R2_BUCKET/$R2_PREFIXO"
