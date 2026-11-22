#!/usr/bin/env bash
# recriacao.test.sh: o next-hosted-provider fica na rede do contêiner do next-server
# (network_mode: service:next-server). Recriar o servidor sem recriar o provedor deixa o
# provedor no namespace antigo, o /ready do servidor em 503 e os convites fora do ar.
# Confere que todo script do pacote que sobe o next-server também recria o provedor.
# Roda no Arthur-PC, sem root nem Docker.
set -euo pipefail
AQUI=$(cd "$(dirname "$0")" && pwd)
falhou=0
for s in "$AQUI"/*.sh; do
  case "$(basename "$s")" in *.test.sh) continue ;; esac
  grep -qE 'up -d[^#]*next-server' "$s" || continue
  if grep -qE 'up -d --force-recreate (next-server )?next-hosted-provider' "$s"; then
    echo "ok: $(basename "$s") recria o provedor"
  else
    echo "FALHA: $(basename "$s") sobe o next-server sem recriar o next-hosted-provider"; falhou=1
  fi
done
exit $falhou
