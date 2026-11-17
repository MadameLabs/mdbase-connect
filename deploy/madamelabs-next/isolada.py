#!/usr/bin/env python3
"""Confere, pela saída de `nft list table`, que cada faixa dada está isolada.

Isolada = coberta pelo conjunto de uma regra `ip saddr { ... } ct state new drop`.
O nft funde faixas vizinhas ao listar (172.31.251.0/24 a 255 vira
172.31.251.0-172.31.255.255), então a conferência é por cobertura, não por texto.
Uso: nft list table ip laboratorio | python3 isolada.py 172.31.249.0/24 [...]
Sai 0 se todas estão cobertas; 1 e diz quais não estão.
"""
import ipaddress
import re
import sys

REGRA = re.compile(r"ip saddr \{([^}]*)\} ct state new drop\s*$")


def intervalos(elementos):
    for el in (e.strip() for e in elementos.split(",")):
        if not el:
            continue
        if "-" in el:
            a, b = el.split("-", 1)
            yield int(ipaddress.IPv4Address(a.strip())), int(ipaddress.IPv4Address(b.strip()))
        else:
            r = ipaddress.IPv4Network(el, strict=False)
            yield int(r.network_address), int(r.broadcast_address)


def coberta(faixa, conjuntos):
    r = ipaddress.IPv4Network(faixa, strict=False)
    ini, fim = int(r.network_address), int(r.broadcast_address)
    return any(any(a <= ini and fim <= b for a, b in c) for c in conjuntos)


def main():
    conjuntos = [list(intervalos(m.group(1))) for m in map(REGRA.search, sys.stdin) if m]
    faltam = [f for f in sys.argv[1:] if not coberta(f, conjuntos)]
    if not conjuntos or faltam:
        print("nao isoladas: " + (", ".join(faltam) if conjuntos else "nenhuma regra de isolamento"), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
