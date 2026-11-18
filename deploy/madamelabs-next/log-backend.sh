#!/usr/bin/env bash
# Troca o backend do next-log no Labs (root, pelo Doctor).
#
#   bash log-backend.sh memoria   só para provas: nada sobrevive a um reinício do next-log
#   bash log-backend.sh pg        backend Postgres (o da instalação)
#
# Por quê: no upstream, o backend Postgres do log-server não tem o leitor de piso de
# exclusão ("PG is a non-launch fallback", runtime/crates/log-server/src/pg.rs:213); toda
# gravação que publica responde `unavailable`. A prova (Task 6 de pilha-next-no-labs) rodou
# com o backend em memória. O fork tem o leitor desde e9f7ae72: para voltar ao Postgres com
# ele, usar log-registro.sh (imagem .2 e LOGSVC_PG_DELETION_REGISTRY), não `pg` daqui.
# Mexe só na linha LOGSVC_BACKEND do /opt/mdbase-next/compose.yml (cópia .antes-log-memoria).
set -euo pipefail
COMPOSE=/opt/mdbase-next/compose.yml
falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || falha "rode como root"
case "${1:-}" in memoria) NOVO=mem ;; pg) NOVO=pg ;; *) falha "uso: log-backend.sh memoria | pg" ;; esac
[ "$(grep -c '^      LOGSVC_BACKEND: ' "$COMPOSE")" = 1 ] || falha "LOGSVC_BACKEND não aparece exatamente uma vez em $COMPOSE"
[ -f "$COMPOSE.antes-log-memoria" ] || cp -p "$COMPOSE" "$COMPOSE.antes-log-memoria"
sed -i "s/^      LOGSVC_BACKEND: .*/      LOGSVC_BACKEND: $NOVO/" "$COMPOSE"
grep -n '^      LOGSVC_BACKEND: ' "$COMPOSE"
docker compose -p mdbase-next -f "$COMPOSE" --project-directory /opt/mdbase-next up -d next-log
for _ in $(seq 30); do [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8801/health)" = 200 ] && break; sleep 1; done
[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8801/health)" = 200 ] || falha "next-log não respondeu /health"
docker logs --tail 3 mdbase-next-next-log-1 2>&1
echo "ok: next-log com backend $NOVO; /health 200"
