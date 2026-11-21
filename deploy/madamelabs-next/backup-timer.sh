#!/usr/bin/env bash
# backup-timer.sh <mdbase-next-backup.timer> [--copiar-para-restauro]
# Root, pelo Doctor. Instala o .timer dado, recarrega o systemd e reinicia o timer; mostra a
# próxima execução e o último backup com sucesso. Com --copiar-para-restauro, copia o backup
# cifrado mais recente para /home/mrlabs/restauro (0600, do mrlabs), para o restaurar-teste.sh.
# O arquivo copiado continua cifrado; a chave não sai de /opt/mdbase-next.
set -euo pipefail

falha() { echo "backup-timer: FALHA: $*" >&2; exit 1; }
[[ $EUID -eq 0 ]] || falha "precisa de root"
[[ $# -ge 1 && -f $1 ]] || falha "uso: backup-timer.sh <mdbase-next-backup.timer> [--copiar-para-restauro]"
TIMER=$1
COPIAR=${2:-}
[[ -z $COPIAR || $COPIAR == --copiar-para-restauro ]] || falha "opção desconhecida: $COPIAR"
grep -qE '^OnCalendar=' "$TIMER" || falha "$TIMER não tem OnCalendar"

DESTINO=/etc/systemd/system/mdbase-next-backup.timer
[[ -f $DESTINO ]] && cp -p "$DESTINO" "$DESTINO.antes.$(date +%Y%m%d-%H%M%S)"
install -m 0644 -o root -g root "$TIMER" "$DESTINO"
systemctl daemon-reload
systemctl restart mdbase-next-backup.timer
systemctl list-timers mdbase-next-backup.timer --no-pager
echo "último sucesso: $(cat /var/backups/mdbase-next/.ultimo-sucesso 2>/dev/null || echo nenhum)"

if [[ $COPIAR == --copiar-para-restauro ]]; then
  u=$(ls -1t /var/backups/mdbase-next/mdbase-next-*.tar.gz.gpg 2>/dev/null | head -n1)
  [[ -n $u ]] || falha "nenhum backup em /var/backups/mdbase-next"
  install -d -m 0700 -o mrlabs -g mrlabs /home/mrlabs/restauro
  install -m 0600 -o mrlabs -g mrlabs "$u" /home/mrlabs/restauro/
  ls -la /home/mrlabs/restauro
fi
