#!/usr/bin/env bash
# Inclui 172.31.249.0/24 (rede da pilha mdbase-next) no define ISOLADAS do
# /etc/nftables/laboratorio-rede.nft e recarrega o laboratorio-rede.service.
# Roda como root, pelo Doctor. Idempotente. Em falha, volta o arquivo e recarrega.
# Uso: bash liberar-rede.sh /home/mrlabs/mdbase-next/isolada.py
# O isolada.py vai como argumento: o Doctor só leva ao preparo (e ao Toxy) o que está no argv.
set -euo pipefail
ARQ=/etc/nftables/laboratorio-rede.nft
FAIXA=172.31.249.0/24
ISOLADA_PY=${1:-$(dirname "$(readlink -f "$0")")/isolada.py}
[ -f "$ISOLADA_PY" ] || { echo "isolada.py nao encontrado: $ISOLADA_PY" >&2; exit 1; }
ISOLADA="python3 -I $ISOLADA_PY"   # confere por cobertura: o nft funde faixas vizinhas ao listar
[ "$(id -u)" = 0 ] || { echo "precisa de root" >&2; exit 1; }
if nft list table ip laboratorio 2>/dev/null | $ISOLADA "$FAIXA" 2>/dev/null; then echo "ja isolada: nada a fazer"; exit 0; fi
grep -q '^define ISOLADAS = {' "$ARQ" || { echo "define ISOLADAS nao encontrado em $ARQ" >&2; exit 1; }
ANTIGAS=$(sed -n 's|^define ISOLADAS = { \(.*\) }$|\1|p' "$ARQ" | tr -d ',')
[ -n "$ANTIGAS" ] || { echo "faixas atuais de ISOLADAS nao lidas" >&2; exit 1; }
BAK="$ARQ.antes-mdbase-next.$(date +%Y%m%d-%H%M%S)"
cp -p "$ARQ" "$BAK"
TMP=$(mktemp)
trap 'rc=$?; rm -f "$TMP"; if [ $rc -ne 0 ]; then cp -p "$BAK" "$ARQ"; systemctl restart laboratorio-rede.service || true; echo "FALHOU: arquivo restaurado de $BAK; laboratorio-rede: $(systemctl is-active laboratorio-rede.service)" >&2; fi' EXIT
if ! grep -qF "$FAIXA" "$ARQ"; then
  sed "s|^define ISOLADAS = { \(.*\) }$|define ISOLADAS = { \1, $FAIXA }|" "$ARQ" > "$TMP"
  grep -q "^define ISOLADAS = {.*$FAIXA }$" "$TMP" || { echo "edicao do define falhou" >&2; exit 1; }
  [ "$(diff "$ARQ" "$TMP" | grep -c '^[<>]')" = 2 ] || { echo "a edicao mudaria mais de uma linha" >&2; exit 1; }
  nft -c -f "$TMP"
  cat "$TMP" > "$ARQ"
fi
systemctl restart laboratorio-rede.service
systemctl is-active --quiet laboratorio-rede.service
# shellcheck disable=SC2086  # ANTIGAS é uma lista de faixas, uma por palavra
nft list table ip laboratorio | $ISOLADA "$FAIXA" $ANTIGAS   # a nova e as antigas, todas isoladas
echo "ok: $FAIXA isolada; backup em $BAK"
