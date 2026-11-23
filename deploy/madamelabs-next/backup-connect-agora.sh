#!/usr/bin/env bash
# Roda agora o backup do Connect atual (mdbase-connect-backup.service) e espera o resultado
# (root, pelo Doctor). Usado no dia da troca da BIBLIOTECA, antes de parar o espelho.
set -euo pipefail
S=mdbase-connect-backup.service
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$EUID" -eq 0 ] || falha "rode como root"
inicio=$(date '+%F %T')
systemctl start --wait "$S" || { journalctl -u "$S" --since "$inicio" --no-pager | tail -15; falha "$S falhou"; }
journalctl -u "$S" --since "$inicio" --no-pager | tail -6
echo "ok: backup do Connect concluído (início $inicio)"
