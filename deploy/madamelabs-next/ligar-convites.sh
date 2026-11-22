#!/usr/bin/env bash
# Liga (ou desliga) as rotas de convite de coleção no next-server do Labs (root, pelo Doctor).
# Idempotente. Uso: ligar-convites.sh ligar | desligar
#
# Uma coleção Private nativa ganha membro de outra conta pelo convite
# (/v1/hosted/collections/:id/invitations). O servidor só monta essas rotas com
# MDBASE_CONNECT_HOSTED_COLLECTIONS=1, MDBASE_CONNECT_HOSTED_SHARING_ENABLED=1 e um provedor
# hosted que responda ao /ready. O provedor é o next-hosted-provider (ligar-provedor.sh, antes
# deste): ele fica na rede do contêiner do servidor, em http://127.0.0.1:8790, que o servidor
# aceita como loopback. O token interno vem do .env.provedor e nunca é impresso.
#
# Recriar o next-server tira a rede do provedor, então os dois são recriados juntos.
# Cópia do .env.servidor antes da mudança: .env.servidor.antes-convites. "desligar" a restaura;
# o provedor continua de pé, ocioso.
set -euo pipefail
ENV_SERVIDOR=/opt/mdbase-next/.env.servidor
ENV_PROVEDOR=/opt/mdbase-next/.env.provedor
COPIA=$ENV_SERVIDOR.antes-convites
C=mdbase-next-next-server-1
CP=mdbase-next-next-hosted-provider-1
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || falha "rode como root"
[ -f "$ENV_SERVIDOR" ] || falha "$ENV_SERVIDOR não existe"
codigo() { curl -s -o /dev/null -m 10 -w '%{http_code}' -X "$1" "http://127.0.0.1:8800$2" || true; }
provedor_pronto() { docker exec "$CP" curl -fsS -o /dev/null -m 10 http://127.0.0.1:8790/ready 2>/dev/null; }
subir() {
	docker compose -p mdbase-next -f /opt/mdbase-next/compose.yml --project-directory /opt/mdbase-next up -d --force-recreate next-server next-hosted-provider
	for _ in $(seq 60); do [ "$(codigo GET /ready)" = 200 ] && provedor_pronto && return 0; sleep 2; done
	return 1
}
logs() {
	for c in "$C" "$CP"; do
		echo "--- $c"
		docker logs --tail 15 "$c" 2>&1 | sed -E 's/((key|secret|token|password)[^=:]*[=:])[^ ,}"]+/\1<oculto>/Ig'
	done
}

case "${1:-}" in
ligar)
	if grep -q '^MDBASE_CONNECT_HOSTED_SHARING_ENABLED=1' "$ENV_SERVIDOR"; then
		echo ".env.servidor já tem os convites ligados"
	else
		grep -qE '^MDBASE_CONNECT_(HOSTED_|DEV_AUTH|ALLOW_INSECURE_HOSTED)' "$ENV_SERVIDOR" \
			&& falha "$ENV_SERVIDOR já tem parte dessas variáveis; conferir à mão"
		[ -f "$ENV_PROVEDOR" ] || falha "$ENV_PROVEDOR não existe; rodar ligar-provedor.sh antes"
		provedor_pronto || falha "o provedor não está com /ready 200; rodar ligar-provedor.sh antes"
		TOKEN=$(grep -E '^MDBASE_CONNECT_HOSTED_PROVIDER_INTERNAL_TOKEN=' "$ENV_PROVEDOR" | cut -d= -f2-)
		[ "${#TOKEN}" -ge 32 ] || falha "token interno ausente ou curto no $ENV_PROVEDOR"
		cp -p "$ENV_SERVIDOR" "$COPIA"
		{
			echo "MDBASE_CONNECT_HOSTED_COLLECTIONS=1"
			echo "MDBASE_CONNECT_HOSTED_SHARING_ENABLED=1"
			echo "MDBASE_CONNECT_HOSTED_PROVIDER_URL=http://127.0.0.1:8790"
			echo "MDBASE_CONNECT_HOSTED_PROVIDER_INTERNAL_TOKEN=$TOKEN"
		} >> "$ENV_SERVIDOR"
		unset TOKEN
		echo ".env.servidor: convites ligados (cópia em $COPIA)"
	fi
	if ! subir; then
		logs
		cp -p "$COPIA" "$ENV_SERVIDOR"
		subir || true
		falha "next-server não voltou com os convites; .env.servidor restaurado"
	fi
	I=$(codigo GET /v1/hosted/collections/00000000-0000-4000-8000-000000000001/invitations)
	echo "sem sessão: GET …/invitations=$I (404 = rota ausente; 401 = montada, pede login)"
	[ "$I" != 404 ] || falha "rota de convite ainda ausente"
	echo "ok: convites montados; /ready 200 no servidor e no provedor"
	;;
desligar)
	[ -f "$COPIA" ] || falha "$COPIA não existe; nada a desligar"
	cp -p "$COPIA" "$ENV_SERVIDOR"
	subir || { logs; falha "next-server ou provedor não voltou depois de desligar"; }
	echo "ok: .env.servidor restaurado de $COPIA; /ready 200 no servidor e no provedor"
	;;
*)
	falha "uso: ligar-convites.sh ligar | desligar"
	;;
esac
