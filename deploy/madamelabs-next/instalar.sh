#!/usr/bin/env bash
# instalar.sh: instala a pilha "mdbase next" no Labs.
# Roda como root, uma vez, pelo Doctor Labs, a partir de /home/mrlabs/mdbase-next (imagens e segredos ficam ali).
# Idempotente: pode rodar de novo. .env.* existentes são mantidos; regras de túnel só são acrescentadas se faltarem.
# Nunca mexe em /opt/mdbase-connect nem no mdbase-connect.service do mrlabs.
set -euo pipefail
umask 077

ORIGEM=/home/mrlabs/mdbase-next
DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CONNECT_ENV=/opt/mdbase-connect/.env
CFG=/etc/cloudflared/config.yml
NFT_TABELA="ip laboratorio"
NFT_SET=ISOLADAS
SUB_REDE=172.31.249.0/24
IP_NFT_TESTE=172.31.249.10
SERVER_TAR=mdbase-next-server-0.1.0-mdbaselabs.1.tar.gz
LOG_TAR=mdbase-next-log-0.1.0-mdbaselabs.1.tar.gz
SERVER_IMG=mdbase-next-server:0.1.0-mdbaselabs.1
LOG_IMG=mdbase-next-log:0.1.0-mdbaselabs.1
SHA_SERVER=3bd785210bdb2cbc11908d9a0d999f50e785969ed192abfb3f1423cf5abe2f5e
SHA_LOG=ab59f79610f5339a86710cdc88c6b54eb0f8d5ab441a59c54e4ca96a21c5e0d1
HOST_PORTAL=next.madamelabs.com.br
HOST_LOG=next-log.madamelabs.com.br
PORTA_SERVER=8800
PORTA_LOG=8801
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CFG_MUDOU=0
CONCLUIDO=0
CFG_BAK=""
TMP_CFG=""
TMP_TLS=""
TMP_ENV=""
TMP_CHECK=""

log() { printf '[instalar] %s\n' "$*"; }
falha() { printf '[instalar] FALHA: %s\n' "$*" >&2; exit 1; }

dc() { docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" "$@"; }

volume_com_rotulo() {
  local nome=$1
  docker volume inspect "${PROJETO}_$nome" >/dev/null 2>&1 || docker volume create \
    --label "com.docker.compose.project=$PROJETO" --label "com.docker.compose.volume=$nome" \
    "${PROJETO}_$nome" >/dev/null
}

espera_http() {
  local url=$1 aceito=$2 limite=$3 t=0 code=""
  until code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$url" || true); [[ $code =~ $aceito ]]; do
    t=$((t + 3))
    (( t >= limite )) && falha "sem resposta esperada de $url (último código: ${code:-nenhum})"
    sleep 3
  done
  log "ok $url ($code)"
}

espera_saudavel() {
  local nome=$1 t=0 id st=""
  id=$(dc ps -q "$nome")
  until st=$(docker inspect -f '{{.State.Health.Status}}' "$id" 2>/dev/null || true); [[ $st == healthy ]]; do
    t=$((t + 3))
    (( t >= 90 )) && falha "$nome não ficou saudável (estado: ${st:-desconhecido})"
    sleep 3
  done
  log "$nome saudável"
}

# Estado final para o log do Doctor: quem responde agora, sem alterar nada.
estado_final() {
  echo "[instalar] estado: cloudflared=$(systemctl is-active cloudflared 2>&1 || true)"
  echo "[instalar] estado: connect /ready=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 https://connect.madamelabs.com.br/ready || true)"
}

# Só alterna o cloudflared quando nada é gravado: o trap desfaz a gravação feita nesta execução.
limpar() {
  local st=$?
  [[ -n $TMP_CFG && -f $TMP_CFG ]] && rm -f "$TMP_CFG"
  [[ -n $TMP_TLS && -d $TMP_TLS ]] && rm -rf "$TMP_TLS"
  [[ -n $TMP_ENV && -f $TMP_ENV ]] && rm -f "$TMP_ENV"
  [[ -n $TMP_CHECK && -f $TMP_CHECK ]] && rm -f "$TMP_CHECK"
  if [[ $CFG_MUDOU -eq 1 && $CONCLUIDO -eq 0 ]]; then
    echo "[instalar] FALHA depois de gravar o config.yml: restaurando $CFG_BAK (cópia desta execução)" >&2
    cp -p "$CFG_BAK" "$CFG" || echo "[instalar] ERRO: não consegui restaurar; restaurar à mão de $CFG_BAK" >&2
    systemctl restart cloudflared || true
    sleep 5
    estado_final >&2
  fi
  exit "$st"
}
trap limpar EXIT

# ---------------------------------------------------------------- 0. pré-checagens (nada é alterado aqui)
[[ $EUID -eq 0 ]] || falha "rode como root (sudo bash instalar.sh)"
for c in docker openssl sha256sum gunzip curl cloudflared systemctl awk nft df; do
  command -v "$c" >/dev/null || falha "comando ausente: $c"
done
for arq in compose.yml backup.sh desfazer.sh mdbase-next-backup.service mdbase-next-backup.timer; do
  [[ -f $SRC_DIR/$arq ]] || falha "arquivo ausente ao lado do instalar.sh: $arq"
done
[[ -f $ORIGEM/imagens/$SERVER_TAR && -f $ORIGEM/imagens/$LOG_TAR ]] || falha "imagens ausentes em $ORIGEM/imagens"
[[ -f $CFG ]] || falha "$CFG não existe"
grep -q '^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:' "$CFG" || falha "regra catch-all não encontrada em $CFG"
docker info >/dev/null 2>&1 || falha "docker não responde"

if [[ ! -f $DESTINO/.env.db ]]; then
  for arq in servidor.env log.env; do [[ -f $ORIGEM/$arq ]] || falha "ausente: $ORIGEM/$arq"; done
  [[ -f $CONNECT_ENV ]] || falha "ausente: $CONNECT_ENV"
  grep -q '^MDBASE_CONNECT_GOOGLE_CLIENT_ID=' "$CONNECT_ENV" || falha "MDBASE_CONNECT_GOOGLE_CLIENT_ID não está em $CONNECT_ENV"
  for k in MDBASE_NEXT_ROOT_PUBLIC_KEY MDBASE_NEXT_POLICY_SIGNING_KEY MDBASE_NEXT_POLICY_KEY_CERT MDBASE_NEXT_LOG_TOKEN_SIGNING_KEY MDBASE_NEXT_LOG_TRANSPORT_KEY MDBASE_CONNECT_AUTH_RATE_LIMIT_SECRET; do
    grep -q "^$k=" "$ORIGEM/servidor.env" || falha "$k ausente em servidor.env"
  done
  for k in LOGSVC_ROOT_KEYS LOGSVC_TOKEN_ISSUERS LOGSVC_URL_SECRET; do
    grep -q "^$k=" "$ORIGEM/log.env" || falha "$k ausente em log.env"
  done
fi

# Sub-rede no isolamento: conferida nas regras CARREGADAS, não no arquivo.
nft get element $NFT_TABELA $NFT_SET "{ $IP_NFT_TESTE }" >/dev/null 2>&1 \
  || falha "$IP_NFT_TESTE não está no set $NFT_SET carregado ($NFT_TABELA). Ficha de isolamento antes (LEIA-ME.md, passo 2)"

if ! docker network inspect "${PROJETO}_next" >/dev/null 2>&1; then
  usadas=$(docker network ls -q | xargs -r docker network inspect --format '{{range .IPAM.Config}}{{.Subnet}} {{end}}' | grep -c '172\.31\.249\.' || true)
  [[ ${usadas:-0} -eq 0 ]] || falha "$SUB_REDE já é usada por outra rede Docker"
fi

# Espaço: pelo menos 10 GiB livres onde o Docker guarda as imagens.
raiz_docker=$(docker info --format '{{.DockerRootDir}}')
livre=$(df -B1 --output=avail "$raiz_docker" | tail -1 | tr -d ' ')
[[ $livre -ge $((10 * 1024 * 1024 * 1024)) ]] || falha "menos de 10 GiB livres em $raiz_docker"

# O validador tem que rejeitar um config quebrado; senão não serve de guarda.
TMP_CHECK=$(mktemp)  # em /tmp (umask 077): não depende de $DESTINO existir
printf 'tunnel: teste\ningress:\n  - hostname: sem-catch-all.exemplo.invalido\n    service: http://127.0.0.1:1\n' > "$TMP_CHECK"
if cloudflared tunnel --config "$TMP_CHECK" ingress validate >/dev/null 2>&1; then
  falha "o cloudflared aceitou um config sem catch-all: o validador não serve de guarda"
fi
rm -f "$TMP_CHECK"; TMP_CHECK=""
log "validador do cloudflared rejeita config quebrado (ok)"

# Antes de tocar no túnel: o Connect e o chat precisam responder agora.
curl -fsS -o /dev/null --max-time 15 https://connect.madamelabs.com.br/ready \
  || falha "connect.madamelabs.com.br/ready não responde agora: nada foi alterado no cloudflared"
code_chat=$(curl -s -o /dev/null -w '%{http_code}' --max-time 15 https://chat.madamelabs.com.br/ || true)
[[ $code_chat =~ ^[23][0-9][0-9]$ ]] || falha "chat.madamelabs.com.br não responde agora (código $code_chat): nada foi alterado no cloudflared"
log "pré-checagens ok (connect e chat respondem)"

# ---------------------------------------------------------------- 1. imagens: sha256 fixado no script, depois docker load
install -d -m 0711 -o root -g root "$DESTINO"
echo "$SHA_SERVER  $ORIGEM/imagens/$SERVER_TAR" | sha256sum -c -
echo "$SHA_LOG  $ORIGEM/imagens/$LOG_TAR" | sha256sum -c -
livre_mb=$(( livre / 1024 / 1024 ))
log "espaço livre em $raiz_docker: ${livre_mb} MiB"
gunzip -c "$ORIGEM/imagens/$SERVER_TAR" | docker load
gunzip -c "$ORIGEM/imagens/$LOG_TAR" | docker load
docker image inspect "$SERVER_IMG" "$LOG_IMG" >/dev/null
log "imagens carregadas"

# ---------------------------------------------------------------- 2. arquivos da pilha e segredos (0600, separados por serviço)
install -m 0644 -o root -g root "$SRC_DIR/compose.yml" "$DESTINO/compose.yml"
install -m 0700 -o root -g root "$SRC_DIR/backup.sh" "$DESTINO/backup.sh"
install -m 0700 -o root -g root "$SRC_DIR/desfazer.sh" "$DESTINO/desfazer.sh"

if [[ ! -f $DESTINO/.env.db ]]; then
  PW=$(openssl rand -hex 24)
  TMP_ENV=$(mktemp "$DESTINO/.env.XXXXXX")
  # .env.db: só a senha do Postgres (o next-db recebe só isto).
  printf 'POSTGRES_PASSWORD=%s\n' "$PW" > "$TMP_ENV"
  mv -f "$TMP_ENV" "$DESTINO/.env.db"; TMP_ENV=""

  # .env.servidor: variáveis do servidor, sem nenhuma LOGSVC_*; DATABASE_URL do banco mdbase_next.
  TMP_ENV=$(mktemp "$DESTINO/.env.XXXXXX")
  {
    grep -v '^LOGSVC_' "$ORIGEM/servidor.env"
    echo
    printf 'DATABASE_URL=postgres://mdbase_next:%s@next-db:5432/mdbase_next?sslmode=verify-full\n' "$PW"
    # Login do Google: o allowlist (MDBASE_CONNECT_ALLOWED_GOOGLE_SUBJECTS) é copiado do Connect por decisão do coordenador.
    grep -E '^(MDBASE_CONNECT_GOOGLE_[A-Z_]+|MDBASE_CONNECT_ALLOWED_GOOGLE_SUBJECTS)=' "$CONNECT_ENV"
  } > "$TMP_ENV"
  mv -f "$TMP_ENV" "$DESTINO/.env.servidor"; TMP_ENV=""

  # .env.log: só LOGSVC_* e a URL do banco de log (banco separado, mdbase_next_log).
  TMP_ENV=$(mktemp "$DESTINO/.env.XXXXXX")
  {
    grep '^LOGSVC_' "$ORIGEM/log.env"
    printf 'LOGSVC_PG_URL=postgres://mdbase_next:%s@next-db:5432/mdbase_next_log?sslmode=verify-full\n' "$PW"
  } > "$TMP_ENV"
  mv -f "$TMP_ENV" "$DESTINO/.env.log"; TMP_ENV=""
  unset PW
  log ".env.db, .env.servidor e .env.log criados (0600)"
else
  log ".env.* já existem: mantidos"
fi
chown root:root "$DESTINO"/.env.*

if [[ ! -f $DESTINO/backup.key ]]; then
  openssl rand -base64 48 > "$DESTINO/backup.key"
  chmod 0600 "$DESTINO/backup.key"
  log "backup.key gerada: guardar no Bitwarden antes do primeiro backup"
fi

# ---------------------------------------------------------------- 3. certificado do Postgres, num volume
volume_com_rotulo next-tls
if ! docker run --rm -v "${PROJETO}_next-tls:/tls" --entrypoint sh postgres:17-alpine@sha256:a77aa70e641402a21411b92b721c58d10f23b50ae5fdd2333efd22d166e1e5f7 -c 'test -f /tls/server.crt' >/dev/null 2>&1; then
  TMP_TLS=$(mktemp -d "$DESTINO/.tls.XXXXXX")
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
    -keyout "$TMP_TLS/server.key" -out "$TMP_TLS/server.crt" -days 3650 \
    -subj "/CN=next-db" \
    -addext "subjectAltName=DNS:next-db" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,digitalSignature,keyEncipherment"
  docker run --rm -v "${PROJETO}_next-tls:/tls" -v "$TMP_TLS:/src:ro" --entrypoint sh postgres:17-alpine@sha256:a77aa70e641402a21411b92b721c58d10f23b50ae5fdd2333efd22d166e1e5f7 -c \
    'cp /src/server.crt /tls/server.crt && cp /src/server.crt /tls/ca.crt && cp /src/server.key /tls/server.key && chown 70:70 /tls/server.key && chmod 600 /tls/server.key && chmod 644 /tls/server.crt /tls/ca.crt'
  rm -rf "$TMP_TLS"; TMP_TLS=""
  log "certificado TLS do Postgres criado no volume next-tls"
else
  log "certificado TLS já existe no volume: mantido"
fi

# ---------------------------------------------------------------- 4. banco, log, migração e serviços
volume_com_rotulo next-db
volume_com_rotulo next-log-objects
dc up -d next-db
espera_saudavel next-db

if [[ "$(dc exec -T next-db psql -U mdbase_next -d mdbase_next -tAc "SELECT 1 FROM pg_database WHERE datname='mdbase_next_log'")" != "1" ]]; then
  dc exec -T next-db psql -v ON_ERROR_STOP=1 -U mdbase_next -d mdbase_next -c 'CREATE DATABASE mdbase_next_log' >/dev/null
  log "banco mdbase_next_log criado"
fi

docker run --rm -u 0 -v "${PROJETO}_next-log-objects:/d" --entrypoint sh "$LOG_IMG" -c 'mkdir -p /d && chown 10001:10001 /d && chmod 700 /d'

# next-server depende de next-migrate (service_completed_successfully): o Compose roda a migração antes.
dc up -d next-server next-log
espera_saudavel next-server
espera_http "http://127.0.0.1:$PORTA_SERVER/ready" '^200$' 120
espera_http "http://127.0.0.1:$PORTA_LOG/health" '^200$' 120

# ---------------------------------------------------------------- 5. CNAMEs ANTES do config.yml
TUNEL=$(awk '$1 == "tunnel:" { print $2; exit }' "$CFG" | tr -d "\"'")
[[ -n $TUNEL ]] || falha "nome do túnel não encontrado em $CFG"

tem_regra() {
  local re=${1//./\\.}  # ponto escapado: o host é literal, não curinga
  grep -qE "^[[:space:]]*-[[:space:]]*hostname:[[:space:]]*[\"']?$re[\"']?[[:space:]]*$" "$CFG"
}
ADD_PORTAL=1; ADD_LOG=1
tem_regra "$HOST_PORTAL" && ADD_PORTAL=0
tem_regra "$HOST_LOG" && ADD_LOG=0

for par in "$HOST_PORTAL:$ADD_PORTAL" "$HOST_LOG:$ADD_LOG"; do
  h=${par%:*}; add=${par##*:}
  [[ $add -eq 1 ]] || { log "regra de $h já existe: CNAME não mexido"; continue; }
  if out=$(cloudflared tunnel route dns "$TUNEL" "$h" 2>&1); then
    log "CNAME $h criado"
  elif grep -qi 'already exists' <<<"$out"; then
    # "already exists" não diz para onde o registro aponta: a conferência é manual (LEIA-ME.md).
    [[ ${CNAME_CONFERIDO:-0} == 1 ]] || falha "CNAME $h já existe e não há regra no config. Conferir no painel que aponta para $TUNEL.cfargotunnel.com e rodar de novo com CNAME_CONFERIDO=1"
    log "CNAME $h já existe; conferido à mão (CNAME_CONFERIDO=1)"
  else
    printf '%s\n' "$out" >&2; falha "route dns $h"
  fi
done

# ---------------------------------------------------------------- 6. config.yml: só as regras que faltam, antes do catch-all
if [[ $ADD_PORTAL -eq 1 || $ADD_LOG -eq 1 ]]; then
  IND=$(awk '/^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:/ { match($0, /^[[:space:]]*/); print substr($0, 1, RLENGTH); exit }' "$CFG")
  [[ -n $IND ]] || falha "não achei o indent da regra catch-all"
  TMP_CFG=$(mktemp "$DESTINO/.cfg.XXXXXX")
  awk -v ind="$IND" -v add1="$ADD_PORTAL" -v h1="$HOST_PORTAL" -v p1="$PORTA_SERVER" \
      -v add2="$ADD_LOG" -v h2="$HOST_LOG" -v p2="$PORTA_LOG" '
    !feito && /^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:/ {
      if (add1 == 1) { print ind "- hostname: " h1; print ind "  service: http://127.0.0.1:" p1 }
      if (add2 == 1) { print ind "- hostname: " h2; print ind "  service: http://127.0.0.1:" p2 }
      feito = 1
    }
    { print }' "$CFG" > "$TMP_CFG"
  esperadas=$(( ADD_PORTAL + ADD_LOG + $(grep -c "hostname: " "$CFG") ))
  [[ $(grep -c "hostname: " "$TMP_CFG") -eq $esperadas ]] || falha "a inserção das regras não bateu a contagem"
  cloudflared tunnel --config "$TMP_CFG" ingress validate || falha "config novo reprovado pelo cloudflared"
  for par in "$HOST_PORTAL:$PORTA_SERVER" "$HOST_LOG:$PORTA_LOG"; do
    h=${par%:*}; p=${par##*:}
    out=$(cloudflared tunnel --config "$TMP_CFG" ingress rule "https://$h" 2>&1 || true)
    grep -q "http://127.0.0.1:$p" <<<"$out" || { printf '%s\n' "$out" >&2; falha "https://$h não cai em 127.0.0.1:$p"; }
    log "regra $h -> 127.0.0.1:$p conferida"
  done

  CFG_BAK="$CFG.antes-mdbase-next.$(date +%Y%m%d-%H%M%S)"
  cp -p "$CFG" "$CFG_BAK"
  log "cópia do config.yml feita: $CFG_BAK"
  CFG_MUDOU=1
  cat "$TMP_CFG" > "$CFG"
  rm -f "$TMP_CFG"; TMP_CFG=""
  # Entre a gravação e o CONCLUIDO=1 qualquer falha cai no trap, que restaura a cópia desta execução.
  systemctl restart cloudflared || true
  systemctl is-active --quiet cloudflared || falha "cloudflared não voltou"
  log "cloudflared reiniciado com as regras"
else
  log "regras do túnel já existem: config.yml intocado"
fi

espera_http "https://connect.madamelabs.com.br/ready" '^200$' 120
espera_http "https://chat.madamelabs.com.br/" '^[23][0-9][0-9]$' 120
espera_http "https://$HOST_PORTAL/ready" '^200$' 120
espera_http "https://$HOST_LOG/health" '^200$' 120
CONCLUIDO=1

# ---------------------------------------------------------------- 7. backup diário (timer)
install -m 0644 -o root -g root "$SRC_DIR/mdbase-next-backup.service" /etc/systemd/system/mdbase-next-backup.service
install -m 0644 -o root -g root "$SRC_DIR/mdbase-next-backup.timer" /etc/systemd/system/mdbase-next-backup.timer
systemctl daemon-reload
systemctl enable --now mdbase-next-backup.timer
log "timer mdbase-next-backup ativo (22:50, horário local)"

# ---------------------------------------------------------------- 8. cópias de segredo na pasta do Doctor: apagar
for f in "$ORIGEM"/*.env; do
  [[ -f $f ]] || continue
  shred -u "$f" 2>/dev/null || rm -f "$f"
done
log "cópias de /home/mrlabs/mdbase-next/*.env apagadas"
estado_final
log "instalação concluída. Próximo passo: guardar backup.key no Bitwarden (LEIA-ME.md)"
