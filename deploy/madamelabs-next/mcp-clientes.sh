#!/usr/bin/env bash
# Lista ou renomeia clientes OAuth do gateway MCP da stack next (root, pelo Doctor).
#
#   bash mcp-clientes.sh                       lista id, nome e data de registro
#   bash mcp-clientes.sh <client_id> <nome>    renomeia (o registro de escritas copia o nome na hora)
#
# Depois de autorizar um agente novo, renomear o cliente para o nome dele (o Hermes se registra
# como "Hermes Agent"). Só mexe em mcp_clients.client_name.
set -euo pipefail
PROJETO=mdbase-next
DESTINO=/opt/mdbase-next
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$EUID" -eq 0 ] || falha "rode como root"
sql() { docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" exec -T next-db psql -v ON_ERROR_STOP=1 -q -U mdbase_next -d mdbase_next_mcp "$@"; }
case $# in
0) sql -c "SELECT id, client_name, created_at FROM mcp_clients ORDER BY created_at" ;;
2)
	[[ $1 =~ ^client_[A-Za-z0-9_-]{8,80}$ ]] || falha "client_id inválido: $1"
	[[ $2 =~ ^[[:alnum:][:space:].-]{1,60}$ ]] || falha "nome inválido: $2"
	n=$(sql -tAc "UPDATE mcp_clients SET client_name = '$2' WHERE id = '$1' RETURNING id" | grep -c .) || true
	[ "$n" = 1 ] || falha "nenhum cliente com id $1"
	echo "ok: $1 agora se chama $2"
	;;
*) falha "uso: mcp-clientes.sh [<client_id> <nome>]" ;;
esac
