#!/usr/bin/env bash
# desfazer.sh: derruba a pilha mdbase next e tira as duas regras de túnel dela.
# Não apaga volumes (dados do Postgres, certificado, objetos do log). Não apaga o .env nem a backup.key.
# O cloudflared não tem comando para apagar CNAME: os dois registros saem pelo painel da Cloudflare (ver saída).
set -euo pipefail
umask 077

DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CFG=/etc/cloudflared/config.yml
CFG_BAK="$CFG.antes-mdbase-next"
HOSTS_NEXT="next.madamelabs.com.br|next-log.madamelabs.com.br"

falha() { printf '[desfazer] FALHA: %s\n' "$*" >&2; exit 1; }
[[ $EUID -eq 0 ]] || falha "rode como root"
[[ -f $DESTINO/compose.yml ]] || falha "$DESTINO/compose.yml não existe"

dc() { docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" "$@"; }

systemctl disable --now mdbase-next-backup.timer 2>/dev/null || true
rm -f /etc/systemd/system/mdbase-next-backup.service /etc/systemd/system/mdbase-next-backup.timer
systemctl daemon-reload

dc down
echo "[desfazer] projeto $PROJETO derrubado (volumes mantidos)"

# Tira só as regras desta pilha; o resto do config.yml (outras mudanças depois da instalação) fica.
TMP=$(mktemp "$DESTINO/.cfg.XXXXXX")
trap 'rm -f "$TMP"' EXIT
awk -v hosts="$HOSTS_NEXT" '
  BEGIN { n = split(hosts, lista, "|"); for (i = 1; i <= n; i++) alvo[lista[i]] = 1 }
  skip { skip = 0; if ($0 ~ /^[[:space:]]*service:/) next }
  {
    linha = $0
    if (match(linha, /-[[:space:]]*hostname:[[:space:]]*/)) {
      nome = substr(linha, RSTART + RLENGTH)
      sub(/[[:space:]]+$/, "", nome)
      if (nome in alvo) { skip = 1; next }
    }
    print
  }' "$CFG" > "$TMP"
cloudflared --config "$TMP" tunnel ingress validate || falha "config sem as regras reprovado; nada foi trocado"
cat "$TMP" > "$CFG"
systemctl restart cloudflared
echo "[desfazer] regras de next e next-log tiradas do config.yml; cloudflared reiniciado"
echo "[desfazer] backup da instalação anterior, se precisar voltar inteiro: $CFG_BAK"
echo "[desfazer] CNAMEs a remover no painel da Cloudflare: next.madamelabs.com.br e next-log.madamelabs.com.br"
echo "[desfazer] para apagar os dados (irreversível, só com autorização): docker volume rm mdbase-next_next-db mdbase-next_next-tls mdbase-next_next-log-objects"
