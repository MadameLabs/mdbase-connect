#!/usr/bin/env bash
# Primeira conta no next.madamelabs.com.br por convite com senha, pelo auth-admin do
# próprio servidor (dentro do contêiner next-server). Roda como root, pelo Doctor.
#
#   bash convite.sh abrir <email>   registro em "invite", senha ligada, convite beta_v1;
#                                   imprime só o link de cadastro (vale uma vez, expira)
#   bash convite.sh fechar          registro de volta em "closed" (senha continua ligada,
#                                   senão a conta criada não entra)
#   bash convite.sh ver             política atual
#
# Por quê: com o registro fechado e o ALLOWED_GOOGLE_SUBJECTS vazio, o login pelo Google
# não cria conta, e um convite só é aceito com o registro em "invite" ou "open".
set -euo pipefail
C=mdbase-next-next-server-1
ENV_SERVIDOR=/opt/mdbase-next/.env.servidor
TERMOS=https://madamelabs.com.br/termos-de-uso/
PRIVACIDADE=https://madamelabs.com.br/politica-de-privacidade/
VERSAO=2026-11-17
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || falha "rode como root"
docker inspect -f '{{.State.Running}}' "$C" 2>/dev/null | grep -qx true || falha "$C não está rodando"
adm() { docker exec -w /app "$C" node services/server/dist/auth-admin-cli.js "$@"; }
campo() { python3 -I -c 'import json,sys; d=json.load(sys.stdin); print(d["policy"][sys.argv[1]])' "$1"; }
politica() { adm policy show; }

case "${1:-}" in
	ver)
		politica | python3 -I -c 'import json,sys; p=json.load(sys.stdin)["policy"]; print("registro=%s senha=%s revisao=%s" % (p["registrationMode"], p["passwordAuthEnabled"], p["revision"]))'
		;;
	abrir)
		EMAIL=${2:-}
		[[ $EMAIL =~ ^[^@[:space:]]+@[^@[:space:]]+\.[^@[:space:]]+$ ]] || falha "uso: convite.sh abrir <email>"
		# Cadastro por convite exige termos e privacidade: os endereços vão no .env.servidor
		# (lidos na partida do servidor) e as versões na política.
		if ! grep -q '^MDBASE_CONNECT_TERMS_URL=' "$ENV_SERVIDOR"; then
			cp -p "$ENV_SERVIDOR" "$ENV_SERVIDOR.antes-termos"
			printf 'MDBASE_CONNECT_TERMS_URL=%s\nMDBASE_CONNECT_PRIVACY_URL=%s\n' "$TERMOS" "$PRIVACIDADE" >> "$ENV_SERVIDOR"
			docker compose -p mdbase-next -f /opt/mdbase-next/compose.yml --project-directory /opt/mdbase-next up -d next-server
			for _ in $(seq 60); do [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8800/ready)" = 200 ] && break; sleep 2; done
			[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8800/ready)" = 200 ] || falha "next-server não voltou depois de ler os termos"
			docker inspect -f '{{.State.Running}}' "$C" | grep -qx true || falha "$C parou"
			echo "servidor reiniciado com os endereços de termos e privacidade"
		fi
		REV=$(politica | campo revision)
		adm policy update --registration invite --password-auth enabled \
			--terms-version "$VERSAO" --privacy-version "$VERSAO" --expected-revision "$REV" \
			--actor arthur --reason "primeira conta da stack mdbaselabs (convite)" >/dev/null
		echo "política: registro=invite, senha=enabled, termos e privacidade $VERSAO (revisão anterior $REV)"
		adm invite create --email "$EMAIL" --actor arthur --reason "primeira conta da stack mdbaselabs" \
			--entitlement-profile beta_v1 --expires-in 3600 --token-output shown \
			| python3 -I -c 'import json,sys; i=json.load(sys.stdin)["invitation"]; print("convite para %s, vence %s" % (i["email"], i["expires_at"])); print("link: " + i.get("invitation_url", "https://next.madamelabs.com.br" + i["invitation_path"]))'
		echo "depois do cadastro: bash convite.sh fechar"
		;;
	fechar)
		REV=$(politica | campo revision)
		adm policy update --registration closed --expected-revision "$REV" \
			--actor arthur --reason "conta criada; registro fechado de novo" >/dev/null
		echo "política: registro=closed (revisão anterior $REV)"
		;;
	*) falha "uso: convite.sh abrir <email> | fechar | ver" ;;
esac
