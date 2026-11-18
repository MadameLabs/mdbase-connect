#!/usr/bin/env bash
# Leva o next-log do Labs para a imagem com o registro de exclusões em Postgres (root, pelo Doctor).
#
#   bash log-registro.sh /home/mrlabs/mdbase-next/imagens/mdbase-next-log-0.1.0-mdbaselabs.2.tar.gz
#
# Por quê: o backend Postgres do upstream não tem produtor do piso de exclusão e recusa toda
# gravação (`unavailable`); por isso o next-log ficou em memória (log-backend.sh memoria). A
# imagem .2 traz o PgDeletionRegistry do fork (commit e9f7ae72), ligado por
# LOGSVC_PG_DELETION_REGISTRY=1.
#
# O que faz: confere o sha256 fixado abaixo, carrega a imagem, copia o compose.yml
# (compose.yml.antes-log-registro.<data>), troca a imagem do log, põe LOGSVC_BACKEND: pg e
# LOGSVC_PG_DELETION_REGISTRY: "1", sobe só o next-log e confere /health, /ready e a linha
# "deletion_registry" no log do contêiner. Se algo falhar, volta o compose.yml e sobe o
# next-log de novo como estava.
#
# Efeito colateral esperado: o conteúdo do log em memória some (é o que acontece em qualquer
# reinício com backend mem). As coleções criadas na prova precisam ser recriadas.
set -euo pipefail
DESTINO=/opt/mdbase-next
COMPOSE=$DESTINO/compose.yml
PROJETO=mdbase-next
CONTEINER=mdbase-next-next-log-1
IMG_ANTIGA=mdbase-next-log:0.1.0-mdbaselabs.1
IMG_NOVA=mdbase-next-log:0.1.0-mdbaselabs.2
SHA_NOVA=412c70fd3866e3d294670b18e24056e2f359ceb3dea184a6ddca9142bb2a34f8
PORTA_LOG=8801

falha() { echo "FALHOU: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || falha "rode como root"
[ $# -eq 1 ] || falha "uso: log-registro.sh <mdbase-next-log-0.1.0-mdbaselabs.2.tar.gz>"
TAR=$1
# O Doctor entrega o argumento como <n>-<nome>; o nome tem de ser o da imagem nova.
b=$(basename "$TAR"); b=${b#[0-9]-}; b=${b#[0-9][0-9]-}
[ "$b" = mdbase-next-log-0.1.0-mdbaselabs.2.tar.gz ] || falha "arquivo inesperado: $TAR"
[ -f "$COMPOSE" ] || falha "$COMPOSE não existe"

# ---------------------------------------------------------------- 1. imagem
echo "$SHA_NOVA  $TAR" | sha256sum -c - || falha "sha256 da imagem não confere"
gunzip -c "$TAR" | docker load
docker image inspect "$IMG_NOVA" >/dev/null || falha "$IMG_NOVA não carregou"

# ---------------------------------------------------------------- 2. compose
[ "$(grep -c "^  image: $IMG_ANTIGA\$\|^  image: $IMG_NOVA\$" "$COMPOSE")" = 1 ] \
  || falha "a imagem do log não aparece exatamente uma vez em $COMPOSE"
[ "$(grep -c '^      LOGSVC_BACKEND: ' "$COMPOSE")" = 1 ] \
  || falha "LOGSVC_BACKEND não aparece exatamente uma vez em $COMPOSE"
BAK="$COMPOSE.antes-log-registro.$(date +%Y%m%d-%H%M%S)"
cp -p "$COMPOSE" "$BAK"
voltar() {
  echo "voltando $COMPOSE de $BAK" >&2
  cp -p "$BAK" "$COMPOSE"
  docker compose -p "$PROJETO" -f "$COMPOSE" --project-directory "$DESTINO" up -d next-log || true
}
trap 'voltar' ERR
sed -i "s|^  image: $IMG_ANTIGA\$|  image: $IMG_NOVA|" "$COMPOSE"
sed -i "s/^      LOGSVC_BACKEND: .*/      LOGSVC_BACKEND: pg/" "$COMPOSE"
grep -q '^      LOGSVC_PG_DELETION_REGISTRY: ' "$COMPOSE" \
  || sed -i 's/^      LOGSVC_BACKEND: pg$/      LOGSVC_BACKEND: pg\n      LOGSVC_PG_DELETION_REGISTRY: "1"/' "$COMPOSE"
docker compose -p "$PROJETO" -f "$COMPOSE" --project-directory "$DESTINO" config -q
diff "$BAK" "$COMPOSE" || true

# ---------------------------------------------------------------- 3. subir e conferir
docker compose -p "$PROJETO" -f "$COMPOSE" --project-directory "$DESTINO" up -d next-log
ok=0
for _ in $(seq 60); do
  if [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORTA_LOG/ready")" = 200 ]; then ok=1; break; fi
  sleep 1
done
[ $ok = 1 ] || falha "next-log não respondeu /ready 200"
[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORTA_LOG/health")" = 200 ] || falha "/health"
docker logs "$CONTEINER" 2>&1 | grep -q '"event":"deletion_registry","backend":"pg"' \
  || falha "o next-log subiu sem o registro de exclusões"
[ "$(docker inspect -f '{{.Config.Image}}' "$CONTEINER")" = "$IMG_NOVA" ] || falha "contêiner não está na $IMG_NOVA"
trap - ERR
docker logs --tail 5 "$CONTEINER" 2>&1
echo "ok: next-log em $IMG_NOVA, backend pg com registro de exclusões; /ready e /health 200; cópia em $BAK"
