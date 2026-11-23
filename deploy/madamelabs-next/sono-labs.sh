#!/usr/bin/env bash
# Adia o sono do Labs só por uma noite, ou religa (root, pelo Doctor).
#
#   bash sono-labs.sh adiar-hoje   para o dormir-labs.timer e agenda a religação dele para
#                                  amanhã às 12:00 (timer temporário do systemd)
#   bash sono-labs.sh voltar       religa o dormir-labs.timer agora
#
# Para operações que não podem ficar pela metade com o Labs dormindo (troca da BIBLIOTECA).
set -euo pipefail
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$EUID" -eq 0 ] || falha "rode como root"
T=dormir-labs.timer
case "${1:-}" in
adiar-hoje)
	amanha=$(date -d tomorrow +%F)
	systemctl stop "$T"
	systemd-run --unit=religar-sono-labs --on-calendar="$amanha 12:00:00" --timer-property=Persistent=true \
		/usr/bin/systemctl start "$T" >/dev/null
	systemctl is-active --quiet "$T" && falha "$T continua ativo"
	echo "ok: sono adiado; $T parado e religação agendada para $amanha 12:00 (religar-sono-labs.timer)"
	;;
voltar)
	systemctl start "$T"
	systemctl stop religar-sono-labs.timer 2>/dev/null || true
	systemctl is-active --quiet "$T" || falha "$T não voltou"
	echo "ok: $T ativo de novo"
	;;
*) falha "uso: sono-labs.sh adiar-hoje | voltar" ;;
esac
