#!/usr/bin/env bash
# Liga as rotas de coleção Private no next-server do Labs (root, pelo Doctor). Idempotente.
#
# - MDBASE_NEXT_PRIVATE_BOOTSTRAP=1: criação de coleção Private e entrada de aparelhos
#   (POST /v1/next/collections/private, que o `mdbase sync enable --mode private` chama).
# - MDBASE_NEXT_CLOUD_COPY_BOOTSTRAP=1: o servidor só monta a listagem GET /v1/next/collections
#   com ele. Exige os serviços de cópia na nuvem (hosted e escrow), que não temos: os endereços
#   são .invalid (nunca resolvem) e os tokens, aleatórios; criar uma cópia na nuvem falha, que é
#   o esperado. É o que o teste local (madamelabs-tasks-local-e2e.mjs) faz com 127.0.0.1:9.
#
# Cópia do .env.servidor antes da mudança: .env.servidor.antes-privado.
set -euo pipefail
ENV_SERVIDOR=/opt/mdbase-next/.env.servidor
C=mdbase-next-next-server-1
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || falha "rode como root"
[ -f "$ENV_SERVIDOR" ] || falha "$ENV_SERVIDOR não existe"
codigo() { curl -s -o /dev/null -m 10 -w '%{http_code}' -X "$1" "http://127.0.0.1:8800$2" || true; }

if grep -q '^MDBASE_NEXT_PRIVATE_BOOTSTRAP=1' "$ENV_SERVIDOR"; then
	echo ".env.servidor já tem as rotas Private ligadas"
else
	grep -qE '^MDBASE_NEXT_(PRIVATE_BOOTSTRAP|CLOUD_COPY_BOOTSTRAP|HOSTED_|ESCROW_)' "$ENV_SERVIDOR" \
		&& falha "$ENV_SERVIDOR já tem parte dessas variáveis; conferir à mão"
	cp -p "$ENV_SERVIDOR" "$ENV_SERVIDOR.antes-privado"
	t() { openssl rand -hex 24; }
	{
		echo "MDBASE_NEXT_PRIVATE_BOOTSTRAP=1"
		echo "MDBASE_NEXT_CLOUD_COPY_BOOTSTRAP=1"
		echo "MDBASE_NEXT_HOSTED_SERVICE_URL=https://hosted.mdbaselabs.invalid"
		echo "MDBASE_NEXT_ESCROW_SERVICE_URL=https://escrow.mdbaselabs.invalid"
		echo "MDBASE_NEXT_HOSTED_SERVICE_TOKEN=$(t)"
		echo "MDBASE_NEXT_ESCROW_SERVICE_TOKEN=$(t)"
		echo "MDBASE_NEXT_HOSTED_INTERNAL_TOKEN=$(t)"
		echo "MDBASE_NEXT_ESCROW_INTERNAL_TOKEN=$(t)"
	} >> "$ENV_SERVIDOR"
	echo ".env.servidor: rotas Private ligadas (cópia em $ENV_SERVIDOR.antes-privado)"
fi

docker compose -p mdbase-next -f /opt/mdbase-next/compose.yml --project-directory /opt/mdbase-next up -d next-server
for _ in $(seq 60); do [ "$(codigo GET /ready)" = 200 ] && break; sleep 2; done
if [ "$(codigo GET /ready)" != 200 ]; then
	docker logs --tail 15 "$C" 2>&1 | sed -E 's/((key|secret|token|password)[^=:]*[=:])[^ ,}"]+/\1<oculto>/Ig'
	cp -p "$ENV_SERVIDOR.antes-privado" "$ENV_SERVIDOR"
	docker compose -p mdbase-next -f /opt/mdbase-next/compose.yml --project-directory /opt/mdbase-next up -d next-server
	falha "next-server não voltou com as rotas Private; .env.servidor restaurado"
fi
L=$(codigo GET /v1/next/collections); P=$(codigo POST /v1/next/collections/private)
echo "sem sessão: GET /v1/next/collections=$L, POST /v1/next/collections/private=$P (404 = rota ausente)"
[ "$L" != 404 ] && [ "$P" != 404 ] || falha "rota ainda ausente"
echo "ok: rotas Private montadas; /ready 200"
