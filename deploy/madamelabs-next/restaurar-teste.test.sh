#!/usr/bin/env bash
# restaurar-teste.test.sh: teste do restaurar-teste.sh, sem R2 nem Labs.
# Monta um backup sintético no formato do backup.sh (três dumps -Fc e contagem.tsv com a mesma
# consulta, lida do próprio backup.sh), cifrado com gpg, e confere:
#   1. o arquivo íntegro restaura com exit 0, inclusive o banco do provedor (mdbase_next_hosted,
#      de outro dono);
#   4. um backup antigo, de dois bancos, ainda restaura, com aviso;
#   2. o arquivo com uma contagem adulterada sai com exit diferente de 0;
#   3. a chave errada sai com exit diferente de 0.
# Roda no Arthur-PC, sem root: precisa de docker e gpg.
set -euo pipefail
umask 077

AQUI=$(cd "$(dirname "$0")" && pwd)
IMAGEM=$(grep -oE 'postgres:17-alpine@sha256:[0-9a-f]{64}' "$AQUI/compose.yml" | head -n1)
eval "$(grep -E '^CONTAGEM_SQL=' "$AQUI/backup.sh")"

TMP=$(mktemp -d)
NOME=mdbase-next-restaurar-fonte-$$
trap 'docker rm -f "$NOME" >/dev/null 2>&1 || true; rm -rf "$TMP"' EXIT

falhou=0
ok() { echo "ok: $*"; }
nok() { echo "FALHA: $*"; falhou=1; }

docker run -d --rm --name "$NOME" -e POSTGRES_USER=mdbase_next -e POSTGRES_DB=mdbase_next \
  -e POSTGRES_PASSWORD="$(head -c 18 /dev/urandom | base64)" "$IMAGEM" >/dev/null
# Pelo TCP interno: durante o initdb o servidor provisório só ouve no socket Unix.
for _ in $(seq 1 60); do
  docker exec "$NOME" pg_isready -h 127.0.0.1 -U mdbase_next -d mdbase_next -q 2>/dev/null && break
  sleep 1
done
ps() { docker exec -i "$NOME" psql -v ON_ERROR_STOP=1 -q -U mdbase_next "$@"; }
ps -d mdbase_next -c 'create database mdbase_next_log'
ps -d mdbase_next -c "create table next_contas(id int primary key); insert into next_contas select generate_series(1,7);
  create table next_vazia(id int);"
ps -d mdbase_next -c "create role mdbase_next_hosted nologin"
ps -d mdbase_next -c 'create database mdbase_next_hosted owner mdbase_next_hosted'
ps -d mdbase_next_hosted -c "set role mdbase_next_hosted; create table hp_colecoes(id int primary key); insert into hp_colecoes select generate_series(1,5);"
ps -d mdbase_next -c "create role mdbase_next_mcp nologin"
ps -d mdbase_next -c 'create database mdbase_next_mcp owner mdbase_next_mcp'
ps -d mdbase_next_mcp -c "set role mdbase_next_mcp; create table mcp_write_audit(id int primary key); insert into mcp_write_audit select generate_series(1,3);"
ps -d mdbase_next_log -c "create table ls_registros(id bigserial primary key, b bytea); insert into ls_registros(b) select '\\x00'::bytea from generate_series(1,123);"

mkdir "$TMP/fonte"
docker exec "$NOME" pg_dump -Fc -U mdbase_next -d mdbase_next > "$TMP/fonte/mdbase_next.dump"
docker exec "$NOME" pg_dump -Fc -U mdbase_next -d mdbase_next_log > "$TMP/fonte/mdbase_next_log.dump"
docker exec "$NOME" pg_dump -Fc -U mdbase_next -d mdbase_next_hosted > "$TMP/fonte/mdbase_next_hosted.dump"
docker exec "$NOME" pg_dump -Fc -U mdbase_next -d mdbase_next_mcp > "$TMP/fonte/mdbase_next_mcp.dump"
{
  for banco in mdbase_next mdbase_next_log mdbase_next_hosted mdbase_next_mcp; do
    echo "== $banco"
    docker exec "$NOME" psql -v ON_ERROR_STOP=1 -U mdbase_next -d "$banco" -tAF $'\t' -c "$CONTAGEM_SQL"
  done
} > "$TMP/fonte/contagem.tsv"
grep -q $'^next_contas\t7$' "$TMP/fonte/contagem.tsv" || { cat "$TMP/fonte/contagem.tsv"; echo "FALHA: contagem da fonte inesperada"; exit 1; }

head -c 32 /dev/urandom | base64 > "$TMP/backup.key"
head -c 32 /dev/urandom | base64 > "$TMP/errada.key"
cifra() { tar -C "$1" -czf - $(cd "$1" && ls) \
  | gpg --batch --yes --pinentry-mode loopback --passphrase-file "$TMP/backup.key" \
        --symmetric --cipher-algo AES256 --output "$2" 2>/dev/null; }
cifra "$TMP/fonte" "$TMP/integro.tar.gz.gpg"
cp -a "$TMP/fonte" "$TMP/adulterado"
sed -i 's/^next_contas\t7$/next_contas\t8/' "$TMP/adulterado/contagem.tsv"
cifra "$TMP/adulterado" "$TMP/adulterado.tar.gz.gpg"
# Backup antigo, de antes do provedor: dois dumps e a contagem sem a seção do terceiro banco.
mkdir "$TMP/antigo"
cp "$TMP/fonte/mdbase_next.dump" "$TMP/fonte/mdbase_next_log.dump" "$TMP/antigo/"
sed '/^== mdbase_next_hosted$/,$d' "$TMP/fonte/contagem.tsv" > "$TMP/antigo/contagem.tsv"
cifra "$TMP/antigo" "$TMP/antigo.tar.gz.gpg"

if bash "$AQUI/restaurar-teste.sh" "$TMP/integro.tar.gz.gpg" "$TMP/backup.key" > "$TMP/s1.log" 2>&1; then
  grep -q $'next_contas\t7' "$TMP/s1.log" && grep -q $'ls_registros\t123' "$TMP/s1.log" \
    && grep -q $'hp_colecoes\t5' "$TMP/s1.log" && grep -q $'mcp_write_audit\t3' "$TMP/s1.log" \
    && ok "íntegro: exit 0 e contagens impressas, com o banco do provedor" || { cat "$TMP/s1.log"; nok "íntegro: exit 0 sem as contagens"; }
else cat "$TMP/s1.log"; nok "íntegro: exit diferente de 0"; fi

if bash "$AQUI/restaurar-teste.sh" "$TMP/adulterado.tar.gz.gpg" "$TMP/backup.key" > "$TMP/s2.log" 2>&1; then
  cat "$TMP/s2.log"; nok "adulterado: exit 0"
elif grep -q 'não bate' "$TMP/s2.log"; then ok "adulterado: recusado pela contagem"
else cat "$TMP/s2.log"; nok "adulterado: recusado por outro motivo"; fi

if bash "$AQUI/restaurar-teste.sh" "$TMP/integro.tar.gz.gpg" "$TMP/errada.key" > "$TMP/s3.log" 2>&1; then
  cat "$TMP/s3.log"; nok "chave errada: exit 0"
elif grep -q 'não decifrou' "$TMP/s3.log"; then ok "chave errada: recusada ao decifrar"
else cat "$TMP/s3.log"; nok "chave errada: recusada por outro motivo"; fi

if bash "$AQUI/restaurar-teste.sh" "$TMP/antigo.tar.gz.gpg" "$TMP/backup.key" > "$TMP/s4.log" 2>&1; then
  grep -q 'sem mdbase_next_hosted' "$TMP/s4.log" && grep -q $'next_contas\t7' "$TMP/s4.log" \
    && ok "antigo: exit 0 com aviso" || { cat "$TMP/s4.log"; nok "antigo: exit 0 sem o aviso"; }
else cat "$TMP/s4.log"; nok "antigo: exit diferente de 0"; fi

docker ps -a --format '{{.Names}}' | grep -q '^mdbase-next-restaurar-teste-' \
  && nok "sobrou contêiner de restauração" || ok "nenhum contêiner de restauração sobrou"

exit $falhou
