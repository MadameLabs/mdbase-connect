#!/usr/bin/env bash
# desfazer.sh: derruba a pilha mdbase next e tira as duas regras de túnel dela.
# Não apaga volumes (dados, certificado, objetos do log), nem .env.*, nem backup.key.
# Tira só as regras desta pilha: o resto do config.yml (mudanças posteriores) fica.
# O cloudflared não tem comando para apagar CNAME: os dois registros saem pelo painel da Cloudflare.
set -euo pipefail
umask 077

DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CFG=/etc/cloudflared/config.yml
HOSTS_NEXT="next.madamelabs.com.br|next-log.madamelabs.com.br"

log() { printf '[desfazer] %s\n' "$*"; }
falha() { printf '[desfazer] FALHA: %s\n' "$*" >&2; exit 1; }
[[ $EUID -eq 0 ]] || falha "rode como root"
[[ -f $DESTINO/compose.yml ]] || falha "$DESTINO/compose.yml não existe"

dc() { docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" "$@"; }

estado_final() {
  log "estado: cloudflared=$(systemctl is-active cloudflared 2>&1 || true)"
  log "estado: connect /ready=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 https://connect.madamelabs.com.br/ready || true)"
}

systemctl disable --now mdbase-next-backup.timer 2>/dev/null || true
rm -f /etc/systemd/system/mdbase-next-backup.service /etc/systemd/system/mdbase-next-backup.timer
systemctl daemon-reload

dc down
log "projeto $PROJETO derrubado (volumes mantidos)"

TUNEL=$(awk '$1 == "tunnel:" { print $2; exit }' "$CFG" | tr -d "\"'")
[[ -n $TUNEL ]] || falha "nome do túnel não encontrado em $CFG"

TMP=$(mktemp "$DESTINO/.cfg.XXXXXX")
trap 'rm -f "$TMP"' EXIT
awk -v hosts="$HOSTS_NEXT" '
  BEGIN { n = split(hosts, lista, "|"); for (i = 1; i <= n; i++) alvo[lista[i]] = 1 }
  skip { skip = 0; if ($0 ~ /^[[:space:]]*service:/) next }
  {
    linha = $0
    if (match(linha, /-[[:space:]]*hostname:[[:space:]]*/)) {
      nome = substr(linha, RSTART + RLENGTH)
      gsub(/["'\''[:space:]]/, "", nome)
      if (nome in alvo) { skip = 1; next }
    }
    print
  }' "$CFG" > "$TMP"

if cmp -s "$CFG" "$TMP"; then
  log "nenhuma regra desta pilha no config.yml: nada a tirar"
  estado_final
  exit 0
fi

cloudflared --config "$TMP" tunnel ingress validate || falha "config sem as regras reprovado: nada foi trocado"
if grep -qE 'hostname:[[:space:]]*"?(next|next-log)\.madamelabs\.com\.br' "$TMP"; then
  falha "ainda há regra desta pilha no config novo: nada foi trocado"
fi

# Cópia datada, feita logo antes da gravação. A restauração usa esta cópia, não uma antiga.
BAK="$CFG.antes-desfazer-mdbase-next.$(date +%Y%m%d-%H%M%S)"
cp -p "$CFG" "$BAK"
log "cópia do config.yml feita: $BAK"
cat "$TMP" > "$CFG"
systemctl restart cloudflared

verifica() {
  local t=0
  until systemctl is-active --quiet cloudflared && curl -fsS -o /dev/null --max-time 10 https://connect.madamelabs.com.br/ready; do
    t=$((t + 3))
    (( t >= 60 )) && return 1
    sleep 3
  done
}

if verifica; then
  log "regras de next e next-log tiradas; cloudflared e connect respondem"
else
  log "FALHA: cloudflared ou connect não respondem com o config novo; restaurando $BAK"
  cp -p "$BAK" "$CFG"
  systemctl restart cloudflared || true
  estado_final
  exit 1
fi

estado_final
log "CNAMEs a remover no painel da Cloudflare: next.madamelabs.com.br e next-log.madamelabs.com.br (túnel $TUNEL)"
log "para apagar os dados (irreversível, só com autorização): docker volume rm ${PROJETO}_next-db ${PROJETO}_next-tls ${PROJETO}_next-log-objects"
