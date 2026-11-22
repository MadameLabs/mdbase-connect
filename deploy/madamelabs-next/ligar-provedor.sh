#!/usr/bin/env bash
# Sobe o provedor hosted da stack next ao lado do next-server (root, pelo Doctor). Idempotente.
#
#   bash ligar-provedor.sh /home/mrlabs/mdbase-next/compose.yml
#
# Por quê: o servidor só liga os convites entre contas com um provedor hosted de verdade
# (design: provedor-hosted-na-stack-next-design). O provedor não guarda coleção nenhuma.
#
# Entradas fixas em /home/mrlabs/mdbase-next (nunca no argv, que o Doctor mostra ao Toxy):
#   imagens/mdbase-next-hosted-provider-0.1.0-mdbaselabs.1.tar.gz  (sha256 fixado abaixo)
#   r2-provedor.env  (0600, escrito pelo Arthur; exatamente MDBASE_CONNECT_R2_ENDPOINT,
#                     MDBASE_CONNECT_R2_BUCKET, MDBASE_CONNECT_R2_ACCESS_KEY_ID e
#                     MDBASE_CONNECT_R2_SECRET_ACCESS_KEY; apagado com shred no fim)
#
# O que faz: confere os sha256 da imagem e do compose.yml; confere que o compose.yml em uso é
# o do pacote anterior ou já este; carrega a imagem; cria o papel e o banco mdbase_next_hosted
# no next-db; gera /opt/mdbase-next/.env.provedor (0600) se faltar; instala o compose.yml
# (cópia compose.yml.antes-provedor.<data>); sobe só o next-hosted-provider e espera o /ready.
# Se o /ready não vier: mostra o log filtrado, tira o provedor, volta o compose.yml e falha.
# Nunca imprime segredo.
set -euo pipefail
umask 077
ORIGEM=/home/mrlabs/mdbase-next
DESTINO=/opt/mdbase-next
PROJETO=mdbase-next
CONTEINER=mdbase-next-next-hosted-provider-1
TAR=$ORIGEM/imagens/mdbase-next-hosted-provider-0.1.0-mdbaselabs.1.tar.gz
IMG=mdbase-next-hosted-provider:0.1.0-mdbaselabs.1
SHA_IMG=8ce4c4830486306bba08a77ca262e82052454891f55c3c11f89fc6c5e69ef97f
# compose.yml anterior: o do pacote em 0c74e1b1 e o instalado no Labs (difere só na 1ª linha,
# "Pilha" em vez de "Stack"). E o deste pacote.
SHA_COMPOSE_ANTES="23b73add204411c266f58f9915a6b1dd45f44a014964487ce9a9cc21ab2d9a1c 317756e9e200cf57b62fb94cf8a6cb15459c8a71bfe3e994bb11d2db2d3fa364"
SHA_COMPOSE_NOVO=e6c028ed476b6a509eacea509d22689d0d834463f74ac2fc079d97db963dc19d
R2=$ORIGEM/r2-provedor.env
ENV_PROVEDOR=$DESTINO/.env.provedor
DATA=$(date +%Y%m%d-%H%M%S)

falha() { echo "FALHOU: $*" >&2; exit 1; }
dc() { docker compose -p "$PROJETO" -f "$DESTINO/compose.yml" --project-directory "$DESTINO" "$@"; }
logs() { docker logs --tail 15 "$CONTEINER" 2>&1 | sed -E 's/((key|secret|token|password)[^=:]*[=:])[^ ,}"]+/\1<oculto>/Ig'; }
psql_db() { dc exec -T next-db psql -v ON_ERROR_STOP=1 -q -U mdbase_next -d mdbase_next "$@"; }

# Modo de teste (ligar-provedor.test.sh): caminhos e hashes sob LIGAR_PROVEDOR_TESTE_RAIZ.
# Nunca vale para root ($EUID é da própria bash).
if [ -n "${LIGAR_PROVEDOR_TESTE_RAIZ:-}" ]; then
	[ "$EUID" -ne 0 ] || falha "o modo de teste nunca roda como root"
	ORIGEM=$LIGAR_PROVEDOR_TESTE_RAIZ/origem
	DESTINO=$LIGAR_PROVEDOR_TESTE_RAIZ/opt
	TAR=$ORIGEM/imagens/$(basename "$TAR")
	R2=$ORIGEM/r2-provedor.env
	ENV_PROVEDOR=$DESTINO/.env.provedor
	SHA_IMG=$TESTE_SHA_IMG
	SHA_COMPOSE_ANTES=$TESTE_SHA_COMPOSE_ANTES
	SHA_COMPOSE_NOVO=$TESTE_SHA_COMPOSE_NOVO
	DONO=()
else
	[ "$EUID" -eq 0 ] || falha "rode como root"
	DONO=(-o root -g root)
fi
[ $# -eq 1 ] || falha "uso: ligar-provedor.sh <compose.yml>"
COMPOSE_NOVO=$1
[ -f "$COMPOSE_NOVO" ] || falha "$COMPOSE_NOVO não existe"
echo "$SHA_COMPOSE_NOVO  $COMPOSE_NOVO" | sha256sum -c --quiet - || falha "sha256 do compose.yml não confere"
ATUAL=$(sha256sum "$DESTINO/compose.yml" | cut -d' ' -f1)
[[ " $SHA_COMPOSE_ANTES " == *" $ATUAL "* ]] || [ "$ATUAL" = "$SHA_COMPOSE_NOVO" ] \
	|| falha "o compose.yml em $DESTINO não é o do pacote anterior; conferir à mão"
[ -f "$TAR" ] || falha "imagem ausente: $TAR"
# A pasta é do mrlabs: confere e carrega de uma cópia do root, para o arquivo não trocar no meio.
TRABALHO=$(mktemp -d)
trap 'rm -rf "$TRABALHO"' EXIT
cp "$TAR" "$TRABALHO/imagem.tar.gz"
echo "$SHA_IMG  $TRABALHO/imagem.tar.gz" | sha256sum -c --quiet - || falha "sha256 da imagem não confere"

# R2: exigido na primeira vez; depois, se o arquivo aparecer de novo, troca as quatro linhas.
# A pasta é do mrlabs: link e arquivo não regular são recusados, e o conteúdo é lido uma vez
# só, para uma cópia do root (R2_LIDO), antes de qualquer uso.
R2_LIDO=""
if [ -e "$R2" ] || [ -L "$R2" ]; then
	[ ! -L "$R2" ] || falha "$R2 é link simbólico; recusado"
	[ -f "$R2" ] || falha "$R2 não é arquivo regular; recusado"
	R2_LIDO=$TRABALHO/r2.env
	cp "$R2" "$R2_LIDO"
	! grep -q $'\r' "$R2_LIDO" || falha "$R2 tem CR (fim de linha do Windows); salvar com LF"
	CHAVES=$(grep -cE '^MDBASE_CONNECT_R2_(ENDPOINT|BUCKET|ACCESS_KEY_ID|SECRET_ACCESS_KEY)=.+$' "$R2_LIDO" || true)
	LINHAS=$(grep -cvE '^\s*$' "$R2_LIDO" || true)
	UNICAS=$(grep -oE '^MDBASE_CONNECT_R2_[A-Z_]+=' "$R2_LIDO" | sort -u | wc -l)
	[ "$CHAVES" = 4 ] && [ "$LINHAS" = 4 ] && [ "$UNICAS" = 4 ] \
		|| falha "$R2 precisa de exatamente as quatro chaves MDBASE_CONNECT_R2_*, uma vez cada"
	grep -qE '^MDBASE_CONNECT_R2_ENDPOINT=https://' "$R2_LIDO" || falha "o endpoint do R2 precisa ser https"
elif [ ! -f "$ENV_PROVEDOR" ]; then
	falha "$R2 não existe (o Arthur escreve, 0600)"
fi

gunzip -c "$TRABALHO/imagem.tar.gz" | docker load >/dev/null
echo "imagem carregada: $IMG"

if [ ! -f "$ENV_PROVEDOR" ]; then
	PW=$(openssl rand -hex 24)
	if [ "$(psql_db -tAc "SELECT 1 FROM pg_roles WHERE rolname='mdbase_next_hosted'")" = 1 ]; then
		printf "ALTER ROLE mdbase_next_hosted LOGIN PASSWORD '%s';\n" "$PW" | psql_db
	else
		printf "CREATE ROLE mdbase_next_hosted LOGIN PASSWORD '%s';\n" "$PW" | psql_db
	fi
	TMP=$(mktemp "$DESTINO/.env.provedor.XXXXXX")
	{
		echo "DATABASE_URL=postgres://mdbase_next_hosted:$PW@next-db:5432/mdbase_next_hosted?sslmode=require"
		echo "HOST=127.0.0.1"
		echo "PORT=8790"
		echo "MDBASE_CONNECT_HOSTED_PROVIDER_INTERNAL_TOKEN=$(openssl rand -hex 24)"
		echo "MDBASE_CONNECT_HOSTED_PROVIDER_MASTER_KEY=$(openssl rand -base64 32)"
		grep -E '^MDBASE_CONNECT_R2_' "$R2_LIDO"
	} > "$TMP"
	unset PW
	chmod 600 "$TMP"
	mv "$TMP" "$ENV_PROVEDOR"
	echo ".env.provedor gerado"
	R2_NOVO=0
elif [ -n "$R2_LIDO" ]; then
	TMP=$(mktemp "$DESTINO/.env.provedor.XXXXXX")
	{ grep -vE '^MDBASE_CONNECT_R2_' "$ENV_PROVEDOR"; grep -E '^MDBASE_CONNECT_R2_' "$R2_LIDO"; } > "$TMP"
	chmod 600 "$TMP"
	cp -p "$ENV_PROVEDOR" "$ENV_PROVEDOR.antes-r2.$DATA"
	mv "$TMP" "$ENV_PROVEDOR"
	R2_NOVO=1
	echo ".env.provedor: credenciais do R2 trocadas (cópia em $ENV_PROVEDOR.antes-r2.$DATA)"
else
	R2_NOVO=0
	echo ".env.provedor já existe; mantido"
fi
if [ "$(psql_db -tAc "SELECT 1 FROM pg_database WHERE datname='mdbase_next_hosted'")" != 1 ]; then
	psql_db -c "CREATE DATABASE mdbase_next_hosted OWNER mdbase_next_hosted"
	echo "banco mdbase_next_hosted criado"
fi

COPIA=""
if [ "$ATUAL" != "$SHA_COMPOSE_NOVO" ]; then
	COPIA=$DESTINO/compose.yml.antes-provedor.$DATA
	cp -p "$DESTINO/compose.yml" "$COPIA"
	install -m 0644 "${DONO[@]}" "$COMPOSE_NOVO" "$DESTINO/compose.yml"
	echo "compose.yml instalado (cópia em $COPIA)"
fi

if [ "$R2_NOVO" = 1 ]; then dc up -d --force-recreate next-hosted-provider; else dc up -d next-hosted-provider; fi
for _ in $(seq 60); do
	docker exec "$CONTEINER" curl -fsS -o /dev/null http://127.0.0.1:8790/ready 2>/dev/null && PRONTO=1 && break
	sleep 2
done
if [ "${PRONTO:-0}" != 1 ]; then
	logs
	dc rm -sf next-hosted-provider || true
	[ -n "$COPIA" ] && cp -p "$COPIA" "$DESTINO/compose.yml"
	falha "o provedor não ficou pronto; compose.yml restaurado (o .env.provedor e o banco ficam)"
fi
if [ -n "$R2_LIDO" ]; then rm -f "$R2"; echo "$R2 apagado"; fi
echo "ok: provedor pronto em 127.0.0.1:8790 na rede do next-server"
