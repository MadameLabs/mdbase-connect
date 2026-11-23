#!/usr/bin/env bash
# Corta ou devolve a escrita pelo gateway MCP antigo (mcp.madamelabs.com.br) (root, pelo Doctor).
#
#   bash gateway-antigo.sh parar    para o contêiner mdbase-connect-mcp-1 (congelamento da troca)
#   bash gateway-antigo.sh voltar   religa o contêiner (volta da troca)
#
# Não mexe em nenhum outro serviço do /opt/mdbase-connect. Imprime a hora, que é a hora do
# congelamento: o mcp_write_audit antigo não pode ter linha depois dela.
set -euo pipefail
C=mdbase-connect-mcp-1
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$EUID" -eq 0 ] || falha "rode como root"
docker inspect "$C" >/dev/null 2>&1 || falha "$C não existe"
case "${1:-}" in
parar)
	docker stop "$C" >/dev/null
	[ "$(docker inspect -f '{{.State.Running}}' "$C")" = false ] || falha "$C continua rodando"
	echo "ok: $C parado às $(date '+%F %T %z') (hora do congelamento)"
	;;
voltar)
	docker start "$C" >/dev/null
	[ "$(docker inspect -f '{{.State.Running}}' "$C")" = true ] || falha "$C não voltou"
	echo "ok: $C rodando de novo às $(date '+%F %T %z')"
	;;
*) falha "uso: gateway-antigo.sh parar | voltar" ;;
esac
