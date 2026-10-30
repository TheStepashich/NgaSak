# NgaSak

CLI-автоматизация Ngate VPN + Sakura 2FA + авторизация на `scp.mos.ru`.

Полный цикл в одном месте:

```
ngate connect -> sakura autopush -> SCP compliance/info -> POST /api/login-er
```

## Возможности

- **Полный прогон (`ngasak run`)**
  - Ngate: статус -> подключение -> ожидание `Online`.
  - Sakura: автоподтверждение пушей по совпадающему SCP-логину.
  - SCP: пауза `post_delay` -> получение `compliance`/`info` от локального агента -> `POST /api/login-er`.
- **Windows-режим Ngate** — нативный automation без внешних скриптов и xdotool:
  - поиск окна клиента (заголовок `КриптоПро NGate`, класс `Qt662QWindowIcon`);
  - активация окна + серия Enter-кликов (fallback: `NGASAK_NGATE_CONNECT_MENU`);
  - probe туннельного адаптера (`CP-TAP-Windows Adapter V9`) для статуса.
- **Перезапуск Sakura**: если локальный агент не отвечает — автоперезапуск служб (systemd / PowerShell), настраиваемо через `NGASAK_SAKURA_RESTART_CMD` или `[sakura] restart_command`.
- **Ретраи SCP**: `scp.attempts` (по умолчанию 3) + автоматический рестарт Sakura и повтор POST с новыми `compliance`/`info`, если агент был "не готов".

## Требования

- **Linux**: CryptoPro NGate (`/opt/cprongate/ngateclient`), D-Bus сессия, локальный агент Sakura на `127.0.0.1:4567`.
- **Windows**: CryptoPro NGate (`ngateclient.exe`, обычно `C:\Program Files\Crypto Pro\NGate\ngateclient.exe`), локальный агент Sakura.

## Сборка

```bash
cargo build --release
```

Для Windows (cross-compile, mingw-w64):

```bash
cargo build --release --target x86_64-pc-windows-gnu
```

Иконка `ngasak.exe` вшивается через `build.rs` + `resources/logo.ico` (компиляция ресурса через `windres`; при необходимости переопределить путь: `WINDOWRES=...`). Под Windows нужен mingw-w64 (например, `x86_64-w64-mingw32-windres`).

## Конфигурация

`~/.config/ngasak/config.toml`:

```toml
[ngate]
binary = "/opt/cprongate/ngateclient"
use_dbus = true

[sakura]
poll_interval = 2.0
check_timeout = 10
restart_command = "..."

[scp]
login_url = "https://scp.mos.ru/api/login-er"
post_delay = 45
attempts = 3
login = "..."ent/` — логи Ngate

[run]
post_command = "..."
```

## Использование

### Полный автопрогон

```bash
ngasak run
```

Дополнительно: `--sakura-wait` (ожидание пуша, default 120), `--skip-ngate`, `--skip-sakura-push`.

### Ngate

```bash
ngasak ngate status
ngasak ngate connect              # с автоподтверждением пуша по логину
ngasak ngate connect --all-pushes # подтверждать все пуши
ngasak ngate connect --no-autopush
ngasak ngate connect-gui          # fallback (xdotool / native command)
```

### Sakura

```bash
ngasak sakura register --qr "sakura://..."
ngasak sakura ping
ngasak sakura check
ngasak sakura creds              # compliance/info
ngasak sakura approve <guid>
ngasak sakura deny <guid>
ngasak sakura history
ngasak sakura poll --interval 5
ngasak sakura autopush           # по логину из конфига/env
ngasak sakura autopush --all
ngasak sakura agent              # проверка локального агента
```

### SCP login-er

```bash
ngasak login-er
ngasak login-er --skip-ngate
ngasak login-er --skip-sakura-push
```

### Конфиг / пароль

```bash
ngasak config show
ngasak config path
ngasak config init
ngasak config set-password
ngasak config clear-password
```

## Как это работает

- **Linux**: Ngate через D-Bus (menu/await credentials), Sakura autopush, SCP login.
- **Windows**: нативный прогон окна NGate (`AttachThreadInput` + Enter-клики, adapter probe), Sakura autopush.
- **SCP**: agent readiness -> попытки (`scp.attempts`) -> рестарт Sakura при необходимости -> повтор POST.

## Структура

```
src/
  main.rs, lib.rs
  cli/        # команды (run, ngate, sakura, login-er, config)
  ngate/      # бэкенды: traits, dbus, linux, windows, gui, status
  sakura/     # agent, client, autopush, register, restart
  scp/        # login-er (compliance/info + retry + service restart)
  config.rs   # TOML конфиг
  certs/      # embedded TLS-сертификаты
tools/        # вспомогательные утилиты для диагностики
resources/   # логотип для иконки exe
```

## License

See [LICENSE](LICENSE).
