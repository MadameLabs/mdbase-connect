# Daemon da stack next no Labs, ao lado do espelho

Instalado em 2026-11-22 (plano de migração, Task 5). Sem root: tudo como `mrlabs`. O `mdbase-connect.service`, o `~/.local/bin/mdbase` e o `~/.local/share/connect` não mudam.

## Peças

| O quê | Onde |
|---|---|
| Binário | `/home/mrlabs/mdbase-next/bin/mdbase-next`, compilado no Arthur-PC do fork em `24cc085b` (`cargo build -j4 --release -p mdbn-daemon --features mdbaselabs`), sha256 `073b68cefc41190e…`, glibc pedida até 2.39 (Labs: 2.43) |
| Unidade | `~/.config/systemd/user/mdbase-next-daemon.service` (cópia em `mdbase-next-daemon.service` deste pacote), `enable --now` |
| Estado | `--state-dir /home/mrlabs/.local/state/mdbase-next/s` (perfil isolado) |
| Connect legado | `MDBASE_CONNECT_HOME=/home/mrlabs/.local/state/mdbase-next/no-connect`, pasta vazia: o perfil isolado só olha o estado do Connect antigo por essa variável, então nunca enxerga o `~/.local/share/connect` |
| Segredos | keyring do `mrlabs` (`gnome-keyring`, `org.freedesktop.secrets`, coleção `login`), com prefixo próprio do perfil |
| Conta | `contato@madamelabs.com` (`account sign-in`, aprovado no navegador) |

## Por que a unidade é escrita à mão

O `mdbase service install` sempre roda o perfil instalado (sem `--state-dir`), cria `mdbase.service` e tem rotina para o `mdbase-connect.service` legado (`daemon/src/service/legacy.rs`). Até o dia da troca, o serviço do espelho não pode mudar.

## Comandos (como `mrlabs`)

```sh
B=/home/mrlabs/mdbase-next/bin/mdbase-next; S=/home/mrlabs/.local/state/mdbase-next/s
XDG_RUNTIME_DIR=/run/user/1000 $B --state-dir $S --json collection list
XDG_RUNTIME_DIR=/run/user/1000 $B --state-dir $S collection join --private <id> <pasta vazia>
systemctl --user status mdbase-next-daemon.service
journalctl --user -u mdbase-next-daemon.service -n 50
```

Pela SSH (Tailscale), o `XDG_RUNTIME_DIR` vem vazio. Passar `/run/user/1000`, igual ao do serviço.

## Estado em 2026-11-22

- Coleção "Prova next migracao" (`fd9326a2-…`) em `/home/mrlabs/mdbase-next/prova-migracao`: `ready`, `online`, `pending 0`, 41 arquivos com o mesmo hash da máquina A do Arthur-PC. Aprovada pela A (código 788532).
- Memória do daemon: 13 MiB em repouso; pico de 292 MiB com a BIBLIOTECA inteira no ensaio do dia (`MemoryMax=512M`, era 384M). Labs com 2,5 GiB disponíveis.
- Ao lado: `connect.madamelabs.com.br/ready` 200 e o espelho da BIBLIOTECA `up to date`.

## Cuidados

- **Keyring depois de reboot:** a coleção `login` estava destrancada desde o boot de 2026-11-15. Depois de um reboot, ela pode voltar trancada até alguém entrar com senha, e o daemon novo (como o Connect atual) fica sem acesso aos segredos (`credential_store_unavailable`). Conferir após qualquer reboot: `busctl --user get-property org.freedesktop.secrets /org/freedesktop/secrets/collection/login org.freedesktop.Secret.Collection Locked`.
- **`join` de coleção Private:** sempre com `--private`. Sem a opção, o CLI tenta a rota de cópia na nuvem e leva `409 not_current_cloud_copy`.
- **Volta:** `systemctl --user disable --now mdbase-next-daemon.service`. O estado em `~/.local/state/mdbase-next` fica.
