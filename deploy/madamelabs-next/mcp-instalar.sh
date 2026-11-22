#!/usr/bin/env bash
# Sobe o gateway MCP da stack next (root, pelo Doctor). Idempotente.
#
#   bash mcp-instalar.sh /home/mrlabs/mdbase-next/compose.yml
#
# Plano de migração, Task 6: https://mcp-next.madamelabs.com.br/mcp, para a Miss Labs e a
# Enola escreverem na stack next com o registro de escritas (mcp_write_audit).
#
# Entradas fixas em /home/mrlabs/mdbase-next (só o compose.yml vai no argv):
#   imagens/mdbase-next-mcp-0.1.0-mdbaselabs.1.tar.gz  (sha256 fixado abaixo; conferida e
#   carregada de uma cópia do root, porque a pasta é do mrlabs)
#
# O que faz, na ordem (nada muda antes das pré-checagens):
#   0. pré-checagens: compose.yml do pacote (sha256) e o em uso (anterior ou este); validador
#      do cloudflared rejeita config quebrado; connect e chat respondem;
#   1. imagem: sha256 e docker load;
#   2. banco mdbase_next_mcp no next-db, papel próprio; /opt/mdbase-next/.env.mcp (0600) se faltar;
#   3. compose.yml instalado (cópia compose.yml.antes-mcp.<data>); next-mcp (com a migração)
#      e espera /health 200 em 127.0.0.1:8802;
#   4. CNAME mcp-next pela API da Cloudflare (token do Doctor) e regra no config.yml do
#      cloudflared (cópia datada), conferida antes de gravar;
#   5. fim só com /mcp público respondendo o desafio OAuth (401 com WWW-Authenticate) e
#      connect.madamelabs.com.br 200.
# Se algo falhar depois de mexer: volta o config.yml do cloudflared, tira o next-mcp e volta o
# compose.yml. O .env.mcp e o banco ficam para a próxima tentativa. Nunca imprime segredo.
set -euo pipefail
umask 077
ORIGEM=/home/mrlabs/mdbase-next
DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CFG=/etc/cloudflared/config.yml
CF_INI=/etc/doctor-labs/cloudflare.ini
HOST_MCP=mcp-next.madamelabs.com.br
PORTA_MCP=8802
TAR=$ORIGEM/imagens/mdbase-next-mcp-0.1.0-mdbaselabs.1.tar.gz
IMG=mdbase-next-mcp:0.1.0-mdbaselabs.1
SHA_IMG=92ad11c05c29879bc02806e79f8fe2c5ab6c9ad9e9ee9da4090296633b97307b
# compose.yml em uso antes deste pacote (o do provedor hosted, fork 24cc085b) e o deste pacote.
SHA_COMPOSE_ANTES=e6c028ed476b6a509eacea509d22689d0d834463f74ac2fc079d97db963dc19d
SHA_COMPOSE_NOVO=277a2d016e214d1862c939b6c2de94eb7944ca1b00eb68739be066aebd77bc9a
ENV_MCP=$DESTINO/.env.mcp
DATA=$(date +%Y%m%d-%H%M%S)

log() { printf '[mcp-instalar] %s\n' "$*"; }
falha() { printf '[mcp-instalar] FALHOU: %s\n' "$*" >&2; exit 1; }
dc() { docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" "$@"; }
psql_db() { dc exec -T next-db psql -v ON_ERROR_STOP=1 -q -U mdbase_next -d mdbase_next "$@"; }
codigo() { curl -s -o /dev/null -m 15 -w '%{http_code}' "$1" || true; }
espera() { # url regex segundos
	local fim=$((SECONDS + $3))
	while ((SECONDS < fim)); do [[ $(codigo "$1") =~ $2 ]] && return 0; sleep 3; done
	return 1
}
logs() { docker logs --tail 15 "$PROJETO-next-mcp-1" 2>&1 | sed -E 's/((key|secret|token|password)[^=:]*[=:]\s*"?)[^ ,}"]+/\1<oculto>/Ig; s#://[^:/@]+:[^@]+@#://<oculto>@#g'; }

TRABALHO=""; CFG_BAK=""; CFG_MUDOU=0; COMPOSE_COPIA=""; SUBIU=0; CONCLUIDO=0
limpar() {
	local st=$?
	if [[ $CONCLUIDO -eq 0 ]]; then
		if [[ $CFG_MUDOU -eq 1 ]]; then
			echo "[mcp-instalar] voltando o config.yml do cloudflared ($CFG_BAK)" >&2
			cp -p "$CFG_BAK" "$CFG" && systemctl restart cloudflared || echo "[mcp-instalar] ERRO: restaurar à mão de $CFG_BAK" >&2
		fi
		if [[ $SUBIU -eq 1 ]]; then
			logs >&2 || true
			dc rm -sf next-mcp next-mcp-migrate >/dev/null 2>&1 || true
		fi
		[[ -n $COMPOSE_COPIA ]] && cp -p "$COMPOSE_COPIA" "$DESTINO/compose.yml"
		echo "[mcp-instalar] estado: connect /ready=$(codigo https://connect.madamelabs.com.br/ready), next /ready=$(codigo https://next.madamelabs.com.br/ready)" >&2
	fi
	[[ -n $TRABALHO ]] && rm -rf "$TRABALHO"
	exit "$st"
}
trap limpar EXIT

# ---------------------------------------------------------------- 0. pré-checagens
[ "$EUID" -eq 0 ] || falha "rode como root"
[ $# -eq 1 ] || falha "uso: mcp-instalar.sh <compose.yml>"
COMPOSE_NOVO=$1
[ -f "$COMPOSE_NOVO" ] || falha "$COMPOSE_NOVO não existe"
echo "$SHA_COMPOSE_NOVO  $COMPOSE_NOVO" | sha256sum -c --quiet - || falha "sha256 do compose.yml não confere"
ATUAL=$(sha256sum "$DESTINO/compose.yml" | cut -d' ' -f1)
[[ $ATUAL == "$SHA_COMPOSE_ANTES" || $ATUAL == "$SHA_COMPOSE_NOVO" ]] \
	|| falha "o compose.yml em $DESTINO não é o do pacote anterior; conferir à mão"
[ -f "$TAR" ] || falha "imagem ausente: $TAR"
for c in docker curl cloudflared systemctl awk python3 openssl sha256sum gunzip; do command -v "$c" >/dev/null || falha "falta $c"; done
TRABALHO=$(mktemp -d)
printf 'tunnel: teste\ningress:\n  - hostname: sem-catch-all.exemplo.invalido\n    service: http://127.0.0.1:1\n' > "$TRABALHO/check.yml"
cloudflared tunnel --config "$TRABALHO/check.yml" ingress validate >/dev/null 2>&1 \
	&& falha "o cloudflared aceitou um config sem catch-all: o validador não serve de guarda"
[[ $(codigo https://connect.madamelabs.com.br/ready) == 200 ]] || falha "connect.madamelabs.com.br/ready não responde agora: nada foi alterado"
[[ $(codigo https://chat.madamelabs.com.br/) =~ ^[23][0-9][0-9]$ ]] || falha "chat.madamelabs.com.br não responde agora: nada foi alterado"
log "pré-checagens ok"

# ---------------------------------------------------------------- 1. imagem
cp "$TAR" "$TRABALHO/imagem.tar.gz"
echo "$SHA_IMG  $TRABALHO/imagem.tar.gz" | sha256sum -c --quiet - || falha "sha256 da imagem não confere"
gunzip -c "$TRABALHO/imagem.tar.gz" | docker load >/dev/null
log "imagem carregada: $IMG"

# ---------------------------------------------------------------- 2. banco e .env.mcp
if [ ! -f "$ENV_MCP" ]; then
	PW=$(openssl rand -hex 24)
	if [[ $(psql_db -tAc "SELECT 1 FROM pg_roles WHERE rolname='mdbase_next_mcp'") == 1 ]]; then
		printf "ALTER ROLE mdbase_next_mcp LOGIN PASSWORD '%s';\n" "$PW" | psql_db
	else
		printf "CREATE ROLE mdbase_next_mcp LOGIN PASSWORD '%s';\n" "$PW" | psql_db
	fi
	TMP=$(mktemp "$DESTINO/.env.mcp.XXXXXX")
	{
		echo "DATABASE_URL=postgres://mdbase_next_mcp:$PW@next-db:5432/mdbase_next_mcp?sslmode=verify-full"
		echo "MDBASE_MCP_MASTER_KEY=$(openssl rand -base64 32)"
	} > "$TMP"
	unset PW
	chmod 600 "$TMP"; mv "$TMP" "$ENV_MCP"
	log ".env.mcp gerado"
else
	log ".env.mcp já existe; mantido"
fi
[[ $(psql_db -tAc "SELECT 1 FROM pg_roles WHERE rolname='mdbase_next_mcp'") == 1 ]] \
	|| falha "o .env.mcp existe mas o papel mdbase_next_mcp não; apagar o .env.mcp e rodar de novo"
if [[ $(psql_db -tAc "SELECT 1 FROM pg_database WHERE datname='mdbase_next_mcp'") != 1 ]]; then
	psql_db -c "CREATE DATABASE mdbase_next_mcp OWNER mdbase_next_mcp"
	log "banco mdbase_next_mcp criado"
fi

# ---------------------------------------------------------------- 3. compose e serviço
if [[ $ATUAL != "$SHA_COMPOSE_NOVO" ]]; then
	COMPOSE_COPIA=$DESTINO/compose.yml.antes-mcp.$DATA
	cp -p "$DESTINO/compose.yml" "$COMPOSE_COPIA"
	install -m 0644 -o root -g root "$COMPOSE_NOVO" "$DESTINO/compose.yml"
	log "compose.yml instalado (cópia em $COMPOSE_COPIA)"
fi
SUBIU=1
dc up -d next-mcp
espera "http://127.0.0.1:$PORTA_MCP/health" '^200$' 120 || falha "next-mcp sem /health 200 em 127.0.0.1:$PORTA_MCP"
log "next-mcp pronto em 127.0.0.1:$PORTA_MCP"

# ---------------------------------------------------------------- 4. CNAME e túnel
TUNEL=$(awk '$1 == "tunnel:" { print $2; exit }' "$CFG" | tr -d "\"'")
[[ -n $TUNEL ]] || falha "nome do túnel não encontrado em $CFG"
re=${HOST_MCP//./\\.}
if grep -qE "^[[:space:]]*-[[:space:]]*hostname:[[:space:]]*[\"']?$re[\"']?[[:space:]]*$" "$CFG"; then
	log "regra de $HOST_MCP já existe no cloudflared: túnel e CNAME intocados"
else
	[[ -f $CF_INI ]] || falha "$CF_INI não existe (token de DNS da Cloudflare)"
	CF_TOKEN=$(sed -n 's/^dns_cloudflare_api_token *= *//p' "$CF_INI" | head -1)
	[[ -n $CF_TOKEN ]] || falha "$CF_INI sem dns_cloudflare_api_token"
	cf() { curl -fsS -m 20 -H @- -H "Content-Type: application/json" "$@" <<<"Authorization: Bearer $CF_TOKEN"; }
	ZONA_ID=$(cf "https://api.cloudflare.com/client/v4/zones?name=madamelabs.com.br" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["result"][0]["id"])') \
		|| falha "zona madamelabs.com.br não encontrada pela API"
	atual=$(cf "https://api.cloudflare.com/client/v4/zones/$ZONA_ID/dns_records?name=$HOST_MCP" \
		| python3 -I -c 'import json,sys; r=json.load(sys.stdin)["result"]; print(";".join(x["type"] + " " + x["content"] for x in r))') \
		|| falha "consulta do DNS de $HOST_MCP falhou"
	if [[ $atual == "CNAME $TUNEL.cfargotunnel.com" ]]; then
		log "CNAME $HOST_MCP já aponta para o túnel"
	elif [[ -z $atual ]]; then
		cf -X POST "https://api.cloudflare.com/client/v4/zones/$ZONA_ID/dns_records" \
			--data "{\"type\":\"CNAME\",\"name\":\"$HOST_MCP\",\"content\":\"$TUNEL.cfargotunnel.com\",\"proxied\":true,\"ttl\":1}" >/dev/null \
			|| falha "criação do CNAME $HOST_MCP falhou"
		log "CNAME $HOST_MCP criado (proxied, -> $TUNEL.cfargotunnel.com)"
	else
		falha "$HOST_MCP já existe com outro destino ($atual); conferir no painel da Cloudflare"
	fi
	unset CF_TOKEN
	IND=$(awk '/^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:/ { match($0, /^[[:space:]]*/); print substr($0, 1, RLENGTH); exit }' "$CFG")
	[[ -n $IND ]] || falha "não achei o indent da regra catch-all"
	awk -v ind="$IND" -v h="$HOST_MCP" -v p="$PORTA_MCP" '
		!feito && /^[[:space:]]*-[[:space:]]*service:[[:space:]]*http_status:/ { print ind "- hostname: " h; print ind "  service: http://127.0.0.1:" p; feito = 1 }
		{ print }' "$CFG" > "$TRABALHO/config.yml"
	[[ $(grep -c "hostname: " "$TRABALHO/config.yml") -eq $(( $(grep -c "hostname: " "$CFG") + 1 )) ]] || falha "a inserção da regra não bateu a contagem"
	cloudflared tunnel --config "$TRABALHO/config.yml" ingress validate >/dev/null || falha "config novo reprovado pelo cloudflared"
	cloudflared tunnel --config "$TRABALHO/config.yml" ingress rule "https://$HOST_MCP" 2>&1 | grep -q "http://127.0.0.1:$PORTA_MCP" \
		|| falha "https://$HOST_MCP não cai em 127.0.0.1:$PORTA_MCP"
	CFG_BAK="$CFG.antes-mcp-next.$DATA"
	cp -p "$CFG" "$CFG_BAK"
	CFG_MUDOU=1
	cat "$TRABALHO/config.yml" > "$CFG"
	systemctl restart cloudflared
	systemctl is-active --quiet cloudflared || falha "cloudflared não voltou"
	log "cloudflared com a regra $HOST_MCP -> 127.0.0.1:$PORTA_MCP (cópia em $CFG_BAK)"
fi

# ---------------------------------------------------------------- 5. conferência final
espera https://connect.madamelabs.com.br/ready '^200$' 120 || falha "connect.madamelabs.com.br/ready não voltou"
espera https://next.madamelabs.com.br/ready '^200$' 120 || falha "next.madamelabs.com.br/ready não voltou"
espera "https://$HOST_MCP/mcp" '^401$' 180 || falha "https://$HOST_MCP/mcp não respondeu 401"
curl -s -o /dev/null -D "$TRABALHO/h" -m 15 "https://$HOST_MCP/mcp" || true
grep -qi '^www-authenticate:' "$TRABALHO/h" || falha "o 401 de /mcp não traz WWW-Authenticate (desafio OAuth)"
CONCLUIDO=1
log "ok: https://$HOST_MCP/mcp responde o desafio OAuth; connect e next 200"
