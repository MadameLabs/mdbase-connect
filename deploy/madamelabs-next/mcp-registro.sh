#!/usr/bin/env bash
# Mostra o registro de escritas do gateway MCP da stack next (root, pelo Doctor). Só leitura.
#
#   bash mcp-registro.sh [--cliente NOME] [--nota TRECHO] [--desde AAAA-MM-DD] [--ate AAAA-MM-DD] [--limite N]
#
# Roda o audit-cli.js do próprio gateway (mcp_write_audit): horário, cliente, ferramenta, nota,
# resultado e código de erro. Sem conteúdo de nota.
set -euo pipefail
C=mdbase-next-next-mcp-1
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$EUID" -eq 0 ] || falha "rode como root"
for a in "$@"; do [[ $a =~ ^[[:alnum:][:space:]._:/@-]*$ ]] || falha "argumento com caractere não aceito: $a"; done
docker inspect -f '{{.State.Running}}' "$C" 2>/dev/null | grep -qx true || falha "$C não está rodando"
docker exec "$C" node services/mcp/dist/audit-cli.js "$@"
