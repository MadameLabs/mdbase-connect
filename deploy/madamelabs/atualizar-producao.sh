#!/usr/bin/env bash
# MadameLabs: troca o Connect de producao do Labs da beta.156 para a beta.210.
#
# Roda como root (pelo Doctor ou pelo Arthur). Ordem: confere, faz backup novo,
# constroi as imagens novas sem tocar na producao e so entao troca. Se algo
# falhar antes da troca, a producao fica como estava. Depois da troca, o banco
# de controle ganha as migracoes 0099 a 0103, que a beta.156 nao entende:
# voltar atras e restaurar o backup do passo 1, nao trocar a versao de volta.
set -euo pipefail
trap 'echo "falhou na linha $LINENO: $BASH_COMMAND" >&2' ERR

NOVA=0.1.0-beta.210
VELHA=0.1.0-beta.156
SRC=/home/mrlabs/repos/.connect210/mdbase-connect
OPT=/opt/mdbase-connect
DC=(docker compose --project-name mdbase-connect --env-file "$OPT/.env"
    -f "$OPT/compose.yml" -f "$OPT/override.yml" --profile hosted --profile mcp)

passo() { printf '\n== %s\n' "$*"; }

passo "0. Conferencias"
# root lendo repositorio do mrlabs: o git recusa sem safe.directory
G=(git --no-optional-locks -c "safe.directory=*" -C "$SRC")
# o codigo construido e exatamente o commit revisado (beta.210 + limites +
# CARGO_BUILD_JOBS + registro de escritas do MCP); por cima dele, so commits
# em deploy/madamelabs/
BASE=748bf3e16dc464c09fba4d5e07e78f56184d20c3
"${G[@]}" diff --quiet HEAD
[ "$("${G[@]}" branch --show-current)" = "madamelabs/limites-v0.1.0-beta.210" ]
"${G[@]}" merge-base --is-ancestor "$BASE" HEAD
[ -z "$("${G[@]}" diff --name-only "$BASE" HEAD -- . ':!deploy/madamelabs')" ]
"${G[@]}" merge-base --is-ancestor "v$NOVA" HEAD
# a producao esta na beta.156 e o compose dela e o do upstream daquela tag
grep -qx "MDBASE_CONNECT_VERSION=$VELHA" "$OPT/.env"
cmp -s "$OPT/compose.yml" <("${G[@]}" show "v$VELHA:deploy/self-host/compose.yml")
# o override prende o MCP com o registro de escritas (mcp e mcp-migrate)
MCP_VELHA="mdbase-connect-mcp:$VELHA-registro"
MCP_NOVA="mdbase-connect-mcp:$NOVA-registro"
LINHA_MCP="image: *[\"']?$MCP_VELHA[\"']? *\$"
n_mcp=$(grep -Ec "$LINHA_MCP" "$OPT/override.yml" || true)
echo "linhas do override com $MCP_VELHA: $n_mcp"
[ "$n_mcp" -ge 1 ]
# espaco para as imagens novas
[ "$(df --output=avail -BG / | tail -1 | tr -dc 0-9)" -ge 20 ]
curl -fsS -o /dev/null https://connect.madamelabs.com.br/ready

passo "1. Backup novo dos bancos (bin/backup) antes de qualquer troca"
inicio=$(date +%s)
"$OPT/bin/backup"
[ "$(stat -c %Y /var/backups/mdbase-connect/.ultimo-sucesso)" -ge "$inicio" ]

passo "2. Imagens $NOVA, construidas a partir de $SRC (a producao segue no ar)"
cd "$SRC/deploy/self-host"
# uma imagem por vez: em paralelo, os pnpm install e o cargo disputavam a rede
# fraca do Labs e estouravam o tempo; ate 3 tentativas cada (o cache do Docker
# pula as camadas que ja deram certo)
for svc in relay-broker connect mcp hosted-provider; do
  for tentativa in 1 2 3; do
    if MDBASE_CONNECT_VERSION=$NOVA docker compose --project-name mdbase-connect-build \
         --env-file "$OPT/.env" -f compose.yml --profile hosted --profile mcp \
         build --build-arg CARGO_BUILD_JOBS=2 "$svc"; then
      break
    fi
    [ "$tentativa" -lt 3 ] || { echo "build de $svc falhou 3 vezes" >&2; exit 1; }
    echo "build de $svc falhou (tentativa $tentativa), de novo em 60 s" >&2
    sleep 60
  done
done
for img in server nats mcp hosted-provider; do
  docker image inspect "mdbase-connect-$img:$NOVA" >/dev/null
done
# o MCP desta branch leva o registro de escritas: mesma convencao de nome da 156
docker tag "mdbase-connect-mcp:$NOVA" "$MCP_NOVA"

passo "3. Troca: compose da beta.210 e versao nova no .env (copias .antes-$NOVA)"
cp -a "$OPT/compose.yml" "$OPT/compose.yml.antes-$NOVA"
cp -a "$OPT/.env" "$OPT/.env.antes-$NOVA"
cp -a "$OPT/override.yml" "$OPT/override.yml.antes-$NOVA"
install -m 0644 "$SRC/deploy/self-host/compose.yml" "$OPT/compose.yml"
sed -i "s/^MDBASE_CONNECT_VERSION=.*/MDBASE_CONNECT_VERSION=$NOVA/" "$OPT/.env"
sed -Ei "s|$LINHA_MCP|image: $MCP_NOVA|" "$OPT/override.yml"
[ "$(grep -c "image: $MCP_NOVA\$" "$OPT/override.yml")" -eq "$n_mcp" ]
cd "$OPT"
imagens=$("${DC[@]}" config --images | sort -u)
echo "$imagens"
# toda imagem nossa tem de ser a nova; o override.yml nao pode prender uma antiga
if echo "$imagens" | grep '^mdbase-connect-' | grep -Eqv ":$NOVA(-registro)?\$"; then
  echo "imagem mdbase-connect fora da $NOVA: desfazendo a troca, nada subiu" >&2
  cp -a "$OPT/compose.yml.antes-$NOVA" "$OPT/compose.yml"
  cp -a "$OPT/.env.antes-$NOVA" "$OPT/.env"
  cp -a "$OPT/override.yml.antes-$NOVA" "$OPT/override.yml"
  exit 1
fi

passo "4. Sobe a versao nova (control-migrate aplica 0099 a 0103)"
"${DC[@]}" up -d

passo "5. Espera os tres enderecos responderem (ate 5 min)"
for i in $(seq 60); do
  if curl -fsS -o /dev/null https://connect.madamelabs.com.br/ready \
     && curl -fsS -o /dev/null https://sync.madamelabs.com.br/ready \
     && curl -fsS -o /dev/null https://mcp.madamelabs.com.br/ready; then
    echo "pronto em $((i * 5)) s"; break
  fi
  [ "$i" -lt 60 ] || { echo "nao respondeu em 5 min: ver ${DC[*]} logs" >&2; exit 1; }
  sleep 5
done
"${DC[@]}" ps --format 'table {{.Service}}\t{{.Image}}\t{{.Status}}'
