#!/usr/bin/env bash
# instalar.sh: instala a pilha "mdbase next" no Labs.
# Roda como root, uma vez, pelo Doctor Labs, a partir de /home/mrlabs/mdbase-next (onde ficam as imagens e os .env).
# Idempotente: pode rodar de novo; o .env e a senha existentes são mantidos.
# Nunca mexe em /opt/mdbase-connect nem no mdbase-connect.service do mrlabs.
set -euo pipefail
umask 077

ORIGEM=/home/mrlabs/mdbase-next
DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CONNECT_ENV=/opt/mdbase-connect/.env
CFG=/etc/cloudflared/config.yml
CFG_BAK="$CFG.antes-mdbase-next"
NFT_CONF=/etc/nftables/laboratorio-rede.nft
SUB_REDE=172.31.249.0/24
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
TMP_CFG=""
TMP_TLS=""
TMP_ENV=""

log() { printf '[instalar] %s\n' "$*"; }
falha() { printf '[instalar] FALHA: %s\n' "$*" >&2; exit 1; }

dc() { docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" "$@"; }

# Volume com os rótulos do Compose, para o projeto reconhecê-lo como próprio.
volume_com_rotulo() {
  local nome=$1
  docker volume inspect "$PROJETO""_$nome" >/dev/null 2>&1 || docker volume create \
    --label "com.docker.compose.project=$PROJETO" --label "com.docker.compose.volume=$nome" \
    "$PROJETO""_$nome" >/dev/null
}

# Espera uma URL responder com código que bata na regex. Falha ao estourar o limite.
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
  local nome=$1 limite=90 t=0 id st=""
  id=$(dc ps -q "$nome")
  until st=$(docker inspect -f '{{.State.Health.Status}}' "$id" 2>/dev/null || true); [[ $st == healthy ]]; do
    t=$((t + 3))
    (( t >= limite )) && falha "$nome não ficou saudável (estado: ${st:-desconhecido})"
    sleep 3
  done
  log "$nome saudável"
}

limpar() {
  local st=$?
  [[ -n $TMP_CFG && -f $TMP_CFG ]] && rm -f "$TMP_CFG"
  [[ -n $TMP_TLS && -d $TMP_TLS ]] && rm -rf "$TMP_TLS"
  [[ -n $TMP_ENV && -f $TMP_ENV ]] && rm -f "$TMP_ENV"
  if [[ $CFG_MUDOU -eq 1 && $CONCLUIDO -eq 0 ]]; then
    echo "[instalar] falha depois de mexer no config.yml: restaurando $CFG_BAK e reiniciando o cloudflared" >&2
    cp -p "$CFG_BAK" "$CFG" || true
    systemctl restart cloudflared || true
  fi
  exit "$st"
}
trap limpar EXIT

# ---------------------------------------------------------------- 0. pré-checagens (nada é alterado aqui)
[[ $EUID -eq 0 ]] || falha "rode como root (sudo bash instalar.sh)"
for c in docker openssl sha256sum gunzip curl cloudflared systemctl awk; do
  command -v "$c" >/dev/null || falha "comando ausente: $c"
done
for arq in compose.yml backup.sh desfazer.sh mdbase-next-backup.service mdbase-next-backup.timer; do
  [[ -f $SRC_DIR/$arq ]] || falha "arquivo ausente ao lado do instalar.sh: $arq"
done
[[ -f $ORIGEM/imagens/$SERVER_TAR && -f $ORIGEM/imagens/$LOG_TAR ]] || falha "imagens ausentes em $ORIGEM/imagens"
[[ -f $CFG ]] || falha "$CFG não existe"
grep -q '^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:' "$CFG" || falha "regra catch-all não encontrada em $CFG"
if [[ ! -f $DESTINO/.env ]]; then
  for arq in servidor.env log.env; do [[ -f $ORIGEM/$arq ]] || falha "ausente: $ORIGEM/$arq"; done
  [[ -f $CONNECT_ENV ]] || falha "ausente: $CONNECT_ENV"
  grep -q '^MDBASE_CONNECT_GOOGLE_CLIENT_ID=' "$CONNECT_ENV" || falha "MDBASE_CONNECT_GOOGLE_CLIENT_ID não está em $CONNECT_ENV"
fi
docker info >/dev/null 2>&1 || falha "docker não responde"
if ! docker network inspect "${PROJETO}_next" >/dev/null 2>&1; then
  usadas=$(docker network ls -q | xargs -r docker network inspect --format '{{range .IPAM.Config}}{{.Subnet}} {{end}}' | grep -c '172\.31\.249\.' || true)
  if [[ ${usadas:-0} -gt 0 ]]; then
    falha "172.31.249.0/24 já é usada por outra rede Docker: escolher outra sub-rede no compose.yml"
  fi
fi
grep -q "$SUB_REDE" "$NFT_CONF" || falha "$SUB_REDE não está em $NFT_CONF (ISOLADAS). Incluir antes, com backup, e recarregar a tabela"
log "pré-checagens ok"

# ---------------------------------------------------------------- 1. imagens: sha256 fixado no script, depois docker load
install -d -m 0711 -o root -g root "$DESTINO"
echo "$SHA_SERVER  $ORIGEM/imagens/$SERVER_TAR" | sha256sum -c -
echo "$SHA_LOG  $ORIGEM/imagens/$LOG_TAR" | sha256sum -c -
gunzip -c "$ORIGEM/imagens/$SERVER_TAR" | docker load
gunzip -c "$ORIGEM/imagens/$LOG_TAR" | docker load
docker image inspect "$SERVER_IMG" "$LOG_IMG" >/dev/null
log "imagens carregadas"

# ---------------------------------------------------------------- 2. arquivos da pilha e .env (0600)
install -m 0644 -o root -g root "$SRC_DIR/compose.yml" "$DESTINO/compose.yml"
install -m 0700 -o root -g root "$SRC_DIR/backup.sh" "$DESTINO/backup.sh"
install -m 0700 -o root -g root "$SRC_DIR/desfazer.sh" "$DESTINO/desfazer.sh"

if [[ ! -f $DESTINO/.env ]]; then
  TMP_ENV=$(mktemp "$DESTINO/.env.XXXXXX")
  {
    cat "$ORIGEM/servidor.env"
    echo
    cat "$ORIGEM/log.env"
    echo
    # Só as variáveis de login do Google, copiadas por nome; os valores não são impressos.
    grep -E '^(MDBASE_CONNECT_GOOGLE_[A-Z_]+|MDBASE_CONNECT_ALLOWED_GOOGLE_SUBJECTS)=' "$CONNECT_ENV"
    echo
    printf 'NEXT_DATABASE_PASSWORD=%s\n' "$(openssl rand -hex 24)"
  } > "$TMP_ENV"
  chmod 0600 "$TMP_ENV"
  mv -f "$TMP_ENV" "$DESTINO/.env"
  TMP_ENV=""
  log ".env criado (0600)"
else
  log ".env já existe: mantido"
fi
chown root:root "$DESTINO/.env"

# Chave de cifragem do backup: gerada uma vez. Precisa ir para o Bitwarden (ver LEIA-ME.md).
if [[ ! -f $DESTINO/backup.key ]]; then
  openssl rand -base64 48 > "$DESTINO/backup.key"
  chmod 0600 "$DESTINO/backup.key"
  log "backup.key gerada: guardar no Bitwarden antes do primeiro backup"
fi

# ---------------------------------------------------------------- 3. certificado do Postgres, num volume
volume_com_rotulo next-tls
if ! docker run --rm -v "$PROJETO""_next-tls:/tls" --entrypoint sh postgres:17-alpine -c 'test -f /tls/server.crt' >/dev/null 2>&1; then
  TMP_TLS=$(mktemp -d "$DESTINO/.tls.XXXXXX")
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
    -keyout "$TMP_TLS/server.key" -out "$TMP_TLS/server.crt" -days 3650 \
    -subj "/CN=next-db" \
    -addext "subjectAltName=DNS:next-db" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,digitalSignature,keyEncipherment"
  # postgres:17-alpine roda como uid 70: a chave precisa ser dele e 0600.
  docker run --rm -v "$PROJETO""_next-tls:/tls" -v "$TMP_TLS:/src:ro" --entrypoint sh postgres:17-alpine -c \
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

# Dono do volume de objetos do log: uid 10001 (usuário logsvc da imagem).
docker run --rm -u 0 -v "$PROJETO""_next-log-objects:/d" --entrypoint sh "$LOG_IMG" -c 'mkdir -p /d && chown 10001:10001 /d && chmod 700 /d'

# next-server depende de next-migrate (service_completed_successfully): o Compose roda a migração antes.
dc up -d next-server next-log
espera_saudavel next-server
espera_http "http://127.0.0.1:$PORTA_SERVER/ready" '^200$' 120
espera_http "http://127.0.0.1:$PORTA_LOG/health" '^200$' 120

# ---------------------------------------------------------------- 5. cloudflared: duas regras antes do catch-all
if grep -q "hostname: $HOST_PORTAL" "$CFG"; then
  log "regras do túnel já existem: mantidas"
else
  [[ -f $CFG_BAK ]] || cp -p "$CFG" "$CFG_BAK"
  IND=$(awk '/^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:/ { match($0, /^[[:space:]]*/); print substr($0, 1, RLENGTH); exit }' "$CFG")
  [[ -n $IND ]] || falha "não achei o indent da regra catch-all"
  TMP_CFG=$(mktemp "$DESTINO/.cfg.XXXXXX")
  awk -v ind="$IND" -v h1="$HOST_PORTAL" -v p1="$PORTA_SERVER" -v h2="$HOST_LOG" -v p2="$PORTA_LOG" '
    !feito && /^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:/ {
      print ind "- hostname: " h1
      print ind "  service: http://127.0.0.1:" p1
      print ind "- hostname: " h2
      print ind "  service: http://127.0.0.1:" p2
      feito = 1
    }
    { print }' "$CFG" > "$TMP_CFG"
  [[ $(grep -c "hostname: $HOST_PORTAL\|hostname: $HOST_LOG" "$TMP_CFG") -eq 2 ]] || falha "a inserção das regras não funcionou"
  cloudflared --config "$TMP_CFG" tunnel ingress validate || falha "config novo reprovado pelo cloudflared"
  CFG_MUDOU=1
  cat "$TMP_CFG" > "$CFG"
  rm -f "$TMP_CFG"; TMP_CFG=""
  TUNEL=$(awk '$1 == "tunnel:" { print $2; exit }' "$CFG")
  [[ -n $TUNEL ]] || falha "nome do túnel não encontrado em $CFG"
  for h in "$HOST_PORTAL" "$HOST_LOG"; do
    out=$(cloudflared tunnel route dns "$TUNEL" "$h" 2>&1) || {
      grep -qi 'already exists' <<<"$out" || { printf '%s\n' "$out" >&2; falha "route dns $h"; }
    }
    log "CNAME $h ok"
  done
  systemctl restart cloudflared
  systemctl is-active --quiet cloudflared || falha "cloudflared não voltou"
  log "cloudflared reiniciado"
fi

espera_http "https://connect.madamelabs.com.br/ready" '^200$' 120
espera_http "https://chat.madamelabs.com.br/" '^[23][0-9][0-9]$' 120
espera_http "https://$HOST_PORTAL/ready" '^200$' 120
espera_http "https://$HOST_LOG/health" '^200$' 120
CONCLUIDO=1

# ---------------------------------------------------------------- 6. backup diário (timer)
install -m 0644 -o root -g root "$SRC_DIR/mdbase-next-backup.service" /etc/systemd/system/mdbase-next-backup.service
install -m 0644 -o root -g root "$SRC_DIR/mdbase-next-backup.timer" /etc/systemd/system/mdbase-next-backup.timer
systemctl daemon-reload
systemctl enable --now mdbase-next-backup.timer
log "timer mdbase-next-backup ativo (22:50, horário local)"

# ---------------------------------------------------------------- 7. cópias de segredo na pasta do Doctor: apagar
for f in "$ORIGEM"/*.env; do
  [[ -f $f ]] || continue
  shred -u "$f" 2>/dev/null || rm -f "$f"
done
log "cópias de /home/mrlabs/mdbase-next/*.env apagadas"
log "instalação concluída. Próximo passo: guardar backup.key no Bitwarden e conferir o LEIA-ME.md"
