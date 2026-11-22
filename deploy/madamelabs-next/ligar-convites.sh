#!/usr/bin/env bash
# Liga (ou desliga) as rotas de convite de coleção no next-server do Labs (root, pelo Doctor).
# Idempotente. Uso: ligar-convites.sh ligar | desligar
#
# Para a prova de contas próprias (migração, Task 4): uma coleção Private nativa ganha membro
# de outra conta pelo convite (/v1/hosted/collections/:id/invitations), e o servidor só monta
# essas rotas com:
# - MDBASE_CONNECT_HOSTED_COLLECTIONS=1 e MDBASE_CONNECT_HOSTED_SHARING_ENABLED=1;
# - um provedor hosted configurado, que não temos. O modo de referência exige autenticação de
#   desenvolvimento, então o provedor aponta para http://127.0.0.1:9, onde nada responde
#   (o runtime-config aceita http em loopback). Criar coleção hosted falha, que é o esperado;
#   o convite e o aceite não chamam o provedor.
#
# Cópia do .env.servidor antes da mudança: .env.servidor.antes-convites. "desligar" a restaura.
set -euo pipefail
ENV_SERVIDOR=/opt/mdbase-next/.env.servidor
COPIA=$ENV_SERVIDOR.antes-convites
C=mdbase-next-next-server-1
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || falha "rode como root"
[ -f "$ENV_SERVIDOR" ] || falha "$ENV_SERVIDOR não existe"
codigo() { curl -s -o /dev/null -m 10 -w '%{http_code}' -X "$1" "http://127.0.0.1:8800$2" || true; }
subir() {
	docker compose -p mdbase-next -f /opt/mdbase-next/compose.yml --project-directory /opt/mdbase-next up -d --force-recreate next-server
	for _ in $(seq 60); do [ "$(codigo GET /ready)" = 200 ] && return 0; sleep 2; done
	return 1
}
logs() { docker logs --tail 15 "$C" 2>&1 | sed -E 's/((key|secret|token|password)[^=:]*[=:])[^ ,}"]+/\1<oculto>/Ig'; }

case "${1:-}" in
ligar)
	if grep -q '^MDBASE_CONNECT_HOSTED_SHARING_ENABLED=1' "$ENV_SERVIDOR"; then
		echo ".env.servidor já tem os convites ligados"
	else
		grep -qE '^MDBASE_CONNECT_(HOSTED_|DEV_AUTH|ALLOW_INSECURE_HOSTED)' "$ENV_SERVIDOR" \
			&& falha "$ENV_SERVIDOR já tem parte dessas variáveis; conferir à mão"
		cp -p "$ENV_SERVIDOR" "$COPIA"
		{
			echo "MDBASE_CONNECT_HOSTED_COLLECTIONS=1"
			echo "MDBASE_CONNECT_HOSTED_SHARING_ENABLED=1"
			echo "MDBASE_CONNECT_HOSTED_PROVIDER_URL=http://127.0.0.1:9"
			echo "MDBASE_CONNECT_HOSTED_PROVIDER_INTERNAL_TOKEN=$(openssl rand -hex 24)"
		} >> "$ENV_SERVIDOR"
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
	echo "ok: convites montados; /ready 200"
	;;
desligar)
	[ -f "$COPIA" ] || falha "$COPIA não existe; nada a desligar"
	cp -p "$COPIA" "$ENV_SERVIDOR"
	subir || { logs; falha "next-server não voltou depois de desligar"; }
	echo "ok: .env.servidor restaurado de $COPIA; /ready 200"
	;;
*)
	falha "uso: ligar-convites.sh ligar | desligar"
	;;
esac
