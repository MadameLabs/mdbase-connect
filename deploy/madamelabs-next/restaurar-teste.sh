#!/usr/bin/env bash
# restaurar-teste.sh <arquivo-do-backup> <backup.key>
# Testa um backup do backup.sh sem tocar em nada: decifra, sobe um Postgres descartável
# (docker run --rm, sem porta publicada), restaura os dois bancos, recalcula a contagem exata
# por tabela com a mesma consulta do backup.sh e compara com o contagem.tsv gravado junto.
# Sai com 0 só se as duas contagens forem iguais. Roda no Arthur-PC, sem root.
# Nenhum segredo é impresso: a chave vai por arquivo ao gpg, a senha do Postgres é aleatória e descartada.
set -euo pipefail
umask 077

falha() { echo "restaurar-teste: FALHA: $*" >&2; exit 1; }

[[ $# -eq 2 ]] || falha "uso: restaurar-teste.sh <arquivo-do-backup> <backup.key>"
ARQUIVO=$1
CHAVE=$2
[[ -f $ARQUIVO ]] || falha "backup não encontrado: $ARQUIVO"
[[ -f $CHAVE ]] || falha "chave não encontrada: $CHAVE"
for c in docker gpg tar diff; do command -v "$c" >/dev/null || falha "comando ausente: $c"; done

AQUI=$(cd "$(dirname "$0")" && pwd)
# A mesma imagem fixada do next-db e a mesma consulta de contagem do backup.sh.
IMAGEM=$(grep -oE 'postgres:17-alpine@sha256:[0-9a-f]{64}' "$AQUI/compose.yml" | head -n1)
[[ -n $IMAGEM ]] || falha "imagem do next-db não encontrada em $AQUI/compose.yml"
linha_sql=$(grep -E '^CONTAGEM_SQL=' "$AQUI/backup.sh") || falha "CONTAGEM_SQL não encontrada em $AQUI/backup.sh"
eval "$linha_sql"

TMP=$(mktemp -d)
NOME=mdbase-next-restaurar-teste-$$
trap 'docker rm -f "$NOME" >/dev/null 2>&1 || true; rm -rf "$TMP"' EXIT

gpg --batch --quiet --pinentry-mode loopback --passphrase-file "$CHAVE" \
    --decrypt --output "$TMP/backup.tar.gz" "$ARQUIVO" 2>"$TMP/gpg.err" \
  || falha "não decifrou (chave errada ou arquivo corrompido): $(tail -n1 "$TMP/gpg.err")"
mkdir "$TMP/x"
tar -C "$TMP/x" -xzf "$TMP/backup.tar.gz" mdbase_next.dump mdbase_next_log.dump contagem.tsv \
  || falha "o arquivo não tem mdbase_next.dump, mdbase_next_log.dump e contagem.tsv"
# Bancos opcionais, cada um do seu papel: o do provedor hosted e o do gateway MCP entraram no
# backup em 2026-11-22; backups anteriores (ou de antes de cada serviço) não os têm.
BANCOS=(mdbase_next mdbase_next_log)
OPCIONAIS=()
for banco in mdbase_next_hosted mdbase_next_mcp; do
  if tar -tzf "$TMP/backup.tar.gz" | grep -qx "$banco.dump"; then
    tar -C "$TMP/x" -xzf "$TMP/backup.tar.gz" "$banco.dump"
    BANCOS+=("$banco"); OPCIONAIS+=("$banco")
  else
    echo "aviso: backup sem $banco; restaurando sem ele"
  fi
done

docker run -d --rm --name "$NOME" --network none \
  -e POSTGRES_USER=mdbase_next -e POSTGRES_DB=mdbase_next \
  -e POSTGRES_PASSWORD="$(head -c 18 /dev/urandom | base64)" "$IMAGEM" >/dev/null \
  || falha "não subiu o Postgres descartável"
# Pelo TCP interno do contêiner: durante o initdb o servidor provisório só ouve no socket Unix.
pronto=0
for _ in $(seq 1 90); do
  if docker exec "$NOME" pg_isready -h 127.0.0.1 -U mdbase_next -d mdbase_next -q 2>/dev/null; then pronto=1; break; fi
  sleep 1
done
[[ $pronto -eq 1 ]] || falha "o Postgres descartável não ficou pronto em 90 s"

docker exec "$NOME" psql -v ON_ERROR_STOP=1 -q -U mdbase_next -d mdbase_next -c 'create database mdbase_next_log' \
  || falha "não criou mdbase_next_log"
for banco in "${OPCIONAIS[@]}"; do
  # Cada banco opcional é do papel de mesmo nome; sem login, só para manter a propriedade.
  docker exec "$NOME" psql -v ON_ERROR_STOP=1 -q -U mdbase_next -d mdbase_next \
    -c "create role $banco nologin" -c "create database $banco owner $banco" \
    || falha "não criou $banco"
done
for banco in "${BANCOS[@]}"; do
  docker exec -i "$NOME" pg_restore --exit-on-error -U mdbase_next -d "$banco" < "$TMP/x/$banco.dump" \
    || falha "pg_restore de $banco falhou"
done

{
  for banco in "${BANCOS[@]}"; do
    echo "== $banco"
    docker exec "$NOME" psql -v ON_ERROR_STOP=1 -U mdbase_next -d "$banco" -tAF $'\t' -c "$CONTAGEM_SQL"
  done
} > "$TMP/restaurado.tsv" || falha "contagem do restaurado falhou"

echo "Contagem por tabela no restaurado (tabela, linhas):"
cat "$TMP/restaurado.tsv"
if diff -u "$TMP/x/contagem.tsv" "$TMP/restaurado.tsv" > "$TMP/diff.txt"; then
  echo "restaurar-teste: OK: a contagem bate com a gravada no backup ($(grep -vc '^==' "$TMP/restaurado.tsv") tabelas)"
else
  echo "Diferença (gravada no backup → restaurado):"
  cat "$TMP/diff.txt"
  falha "a contagem do restaurado não bate com a gravada no backup"
fi
