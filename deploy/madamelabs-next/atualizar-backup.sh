#!/usr/bin/env bash
# Instala um backup.sh novo em /opt/mdbase-next (root, pelo Doctor). Idempotente.
#
#   bash atualizar-backup.sh /home/mrlabs/mdbase-next/backup.sh
#
# Só o instalar.sh instalava o backup.sh, e rodá-lo de novo mexe em muito mais coisa. Este
# troca só o backup.sh: guarda o atual como backup.sh.antes.<data>, instala o novo (0700 root)
# e confere a sintaxe do instalado. O backup.sh de 2026-11-22 faz dump também do banco
# mdbase_next_hosted, então este script recusa se o banco ainda não existe (ligar-provedor.sh
# antes), para o backup das 21:30 não quebrar.
set -euo pipefail
DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
DATA=$(date +%Y%m%d-%H%M%S)
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || falha "rode como root"
[ $# -eq 1 ] || falha "uso: atualizar-backup.sh <backup.sh>"
NOVO=$1
[ -f "$NOVO" ] || falha "$NOVO não existe"
bash -n "$NOVO" || falha "o backup.sh novo tem erro de sintaxe"

if grep -q 'mdbase_next_hosted' "$NOVO"; then
	EXISTE=$(docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" \
		exec -T next-db psql -U mdbase_next -d mdbase_next -tAc \
		"SELECT 1 FROM pg_database WHERE datname='mdbase_next_hosted'" 2>/dev/null || true)
	[ "$EXISTE" = 1 ] || falha "o banco mdbase_next_hosted não existe; rodar ligar-provedor.sh antes"
fi

if [ -f "$DESTINO/backup.sh" ] && cmp -s "$NOVO" "$DESTINO/backup.sh"; then
	echo "backup.sh já é este; nada a fazer"
	exit 0
fi
[ -f "$DESTINO/backup.sh" ] && cp -p "$DESTINO/backup.sh" "$DESTINO/backup.sh.antes.$DATA"
install -m 0700 -o root -g root "$NOVO" "$DESTINO/backup.sh"
bash -n "$DESTINO/backup.sh" || falha "o backup.sh instalado tem erro de sintaxe"
echo "ok: backup.sh instalado (cópia em $DESTINO/backup.sh.antes.$DATA)"
