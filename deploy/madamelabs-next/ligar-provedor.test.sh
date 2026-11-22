#!/usr/bin/env bash
# ligar-provedor.test.sh: teste do ligar-provedor.sh, sem root, sem Docker e sem Labs.
# Roda o script em modo de teste (LIGAR_PROVEDOR_TESTE_RAIZ, que recusa root), com docker e
# gunzip simulados no PATH, e confere:
#   1. r2-provedor.env que é link simbólico é recusado, e o alvo do link fica intacto;
#   2. r2-provedor.env com CR (fim de linha do Windows) é recusado;
#   3. a imagem é conferida e carregada de uma cópia fora da pasta do mrlabs;
#   4. rodar de novo com .env.provedor existente e r2-provedor.env novo troca as quatro linhas
#      do R2, mantém o resto, recria o provedor e apaga o r2-provedor.env;
#   5. o compose.yml do pacote pode ser qualquer versão da lista aceita (provedor, gateway MCP).
set -euo pipefail
AQUI=$(cd "$(dirname "$0")" && pwd)
RAIZ=$(mktemp -d)
trap 'rm -rf "$RAIZ"' EXIT
falhou=0
ok() { echo "ok: $*"; }
nok() { echo "FALHA: $*"; falhou=1; }

STUB=$RAIZ/stub
mkdir -p "$STUB/bin" "$RAIZ/origem/imagens" "$RAIZ/opt"
cat > "$STUB/bin/docker" <<'EOS'
#!/usr/bin/env bash
S=$(dirname "$(dirname "$0")")
echo "docker $*" >> "$S/chamadas"
case " $* " in
  *" load"*) cat > /dev/null; echo "Loaded image" ;;
  *" exec -T next-db psql"*)
    entrada=""; case " $* " in *" -c "*|*" -tAc "*) ;; *) entrada=$(cat) ;; esac
    case "$entrada" in *"CREATE ROLE"*) touch "$S/papel" ;; esac
    case " $* " in
      *"pg_roles"*) [ -f "$S/papel" ] && echo 1 || true ;;
      *"pg_database"*) [ -f "$S/banco" ] && echo 1 || true ;;
      *"CREATE DATABASE"*) touch "$S/banco" ;;
    esac ;;
  *" exec "*"curl"*) exit 0 ;;
esac
exit 0
EOS
cat > "$STUB/bin/gunzip" <<'EOS'
#!/usr/bin/env bash
S=$(dirname "$(dirname "$0")")
echo "${@: -1}" >> "$S/gunzip"
exec /usr/bin/gunzip "$@"
EOS
chmod +x "$STUB/bin/docker" "$STUB/bin/gunzip"

TAR=$RAIZ/origem/imagens/mdbase-next-hosted-provider-0.1.0-mdbaselabs.1.tar.gz
echo "imagem de teste" | gzip > "$TAR"
printf 'name: teste\n' > "$RAIZ/opt/compose.yml"
cp "$AQUI/compose.yml" "$RAIZ/compose-novo.yml"
r2() {
  printf '%s\n' "MDBASE_CONNECT_R2_ENDPOINT=https://conta.r2.cloudflarestorage.com" "MDBASE_CONNECT_R2_BUCKET=mdbase-next-hosted" \
    "MDBASE_CONNECT_R2_ACCESS_KEY_ID=$1" "MDBASE_CONNECT_R2_SECRET_ACCESS_KEY=$2" > "$RAIZ/origem/r2-provedor.env"
}
roda() {
  LISTA_NOVO=${LISTA_NOVO:-} env PATH="$STUB/bin:$PATH" LIGAR_PROVEDOR_TESTE_RAIZ="$RAIZ" \
    TESTE_SHA_IMG="$(sha256sum "$TAR" | cut -d' ' -f1)" \
    TESTE_SHA_COMPOSE_ANTES="$(sha256sum "$RAIZ/opt/compose.yml" | cut -d' ' -f1)" \
    TESTE_SHA_COMPOSE_NOVO="${LISTA_NOVO:-$(sha256sum "$RAIZ/compose-novo.yml" | cut -d' ' -f1)}" \
    bash "$AQUI/ligar-provedor.sh" "$RAIZ/compose-novo.yml" > "$RAIZ/saida.log" 2>&1
}

# 1. link simbólico
echo "alvo intacto" > "$RAIZ/alvo"
ln -s "$RAIZ/alvo" "$RAIZ/origem/r2-provedor.env"
if roda; then nok "link: exit 0"
elif grep -q 'link' "$RAIZ/saida.log" && [ "$(cat "$RAIZ/alvo")" = "alvo intacto" ]; then ok "link: recusado e alvo intacto"
else cat "$RAIZ/saida.log"; nok "link: recusado por outro motivo ou alvo mexido"; fi
rm -f "$RAIZ/origem/r2-provedor.env"

# 2. CR
r2 AAAA BBBB; sed -i 's/$/\r/' "$RAIZ/origem/r2-provedor.env"
if roda; then nok "CR: exit 0"
elif grep -q 'CR' "$RAIZ/saida.log"; then ok "CR: recusado"
else cat "$RAIZ/saida.log"; nok "CR: recusado por outro motivo"; fi
rm -f "$RAIZ/origem/r2-provedor.env"

# 3. primeira execução: imagem carregada de cópia fora da pasta do mrlabs
r2 PRIMEIRA-CHAVE PRIMEIRO-SEGREDO
if roda; then
  carregado=$(tail -n1 "$STUB/gunzip" 2>/dev/null || true)
  if [ -n "$carregado" ] && [[ "$carregado" != "$RAIZ/origem/"* ]]; then ok "imagem: carregada de cópia ($carregado)"
  else nok "imagem: carregada da pasta do mrlabs ($carregado)"; fi
else cat "$RAIZ/saida.log"; nok "primeira execução: exit diferente de 0"; fi

# 4. nova execução com R2 novo
r2 SEGUNDA-CHAVE SEGUNDO-SEGREDO
TOKEN_ANTES=$(grep '^MDBASE_CONNECT_HOSTED_PROVIDER_INTERNAL_TOKEN=' "$RAIZ/opt/.env.provedor" 2>/dev/null || true)
: > "$STUB/chamadas"
if roda; then
  E=$RAIZ/opt/.env.provedor
  if grep -qx 'MDBASE_CONNECT_R2_ACCESS_KEY_ID=SEGUNDA-CHAVE' "$E" && ! grep -q PRIMEIRA "$E" \
    && [ "$(grep -c '^MDBASE_CONNECT_R2_' "$E")" = 4 ] && [ -n "$TOKEN_ANTES" ] && grep -qx "$TOKEN_ANTES" "$E" \
    && grep -q 'up -d --force-recreate next-hosted-provider' "$STUB/chamadas" && [ ! -e "$RAIZ/origem/r2-provedor.env" ]; then
    ok "R2 novo: linhas trocadas, resto mantido, provedor recriado, arquivo apagado"
  else cat "$RAIZ/saida.log"; nok "R2 novo: não aplicado como esperado"; fi
else cat "$RAIZ/saida.log"; nok "R2 novo: exit diferente de 0"; fi

# 5. lista de versões aceitas do compose.yml
LISTA_NOVO="0000000000000000000000000000000000000000000000000000000000000000 $(sha256sum "$RAIZ/compose-novo.yml" | cut -d' ' -f1)"
if LISTA_NOVO=$LISTA_NOVO roda; then ok "lista de compose aceitos: versão da lista aceita"
else cat "$RAIZ/saida.log"; nok "lista de compose aceitos: recusou uma versão da lista"; fi
unset LISTA_NOVO

grep -rqE 'PRIMEIRO-SEGREDO|SEGUNDO-SEGREDO' "$RAIZ/saida.log" && nok "segredo impresso na saída" || ok "nenhum segredo na saída"
exit $falhou
