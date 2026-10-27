# NgaSak

CLI-автоматизация Ngate + Sakura 2FA + авторизация на scp.mos.ru. Полностью заменяет bash/python-скрипты из `sakura-2fa/scripts/`.

## Возможности

- Автоподключение Ngate через D-Bus (с обходом залипания в `AwaitingCredentials`)
- Автоподтверждение пушей Sakura 2FA только по совпадающему логину
- Получение `compliance`/`info` от локального агента Sakura (`127.0.0.1:4567`)
- Авторизация на `scp.mos.ru/api/login-er` с правильной TLS-цепочкой
- Полный паритет с `sakura2fa.py`: `register`, `ping`, `check`, `approve`, `deny`, `history`, `poll`, `autopush`
- Хранение пароля SCP в системном keyring (Secret Service/KWallet)

## Требования

- Linux (x11/wayland)
- [КриптоПро NGate](https://www.cryptopro.ru/products/ngate) установлен в `/opt/cprongate/ngateclient`
- Локальный агент Sakura запущен на `127.0.0.1:4567`
- Настроенный клиент Sakura 2FA с конфигом `~/.config/sakura2fa/config.json`
- Доступ к D-Bus сессии

## Сборка

```bash
cd ~/Programs/ngasak
cargo build --release
cp target/release/ngasak ~/.local/bin/ngasak
```

## Конфигурация

Основной конфиг: `~/.config/ngasak/config.toml`

```toml
[ngate]
binary = "/opt/cprongate/ngateclient"
use_dbus = true

[sakura]
poll_interval = 2.0
check_timeout = 10
server_address = "https://mapp2fasak-m.mos.ru/"
server_uuid = "800ddb8a-8c8c-43d0-9095-bc9e17904d36"
device_token = ""
auth_token = ""
server_name = "DIT-2"
[scp]
login_url = "https://scp.mos.ru/api/login-er"
post_delay = 45
login = "ShichkovSD"

[run]
post_command = "/home/thestepashich/Programs/KSU/AUTOSTART.sh g1"

```

### Сохранение пароля SCP

Пароль можно положить в системный брелок (больше не запрашивается):

```bash
ngasak config set-password
# или напрямую через secret-tool/keyring
```

Проверить:

```bash
secret-tool lookup service ngasak account scp
```

Удалить:

```bash
ngasak config clear-password
```

Порядок получения пароля:
1. `SCP_PASS` из окружения
2. Системный keyring (`service=ngasak, account=scp`, формат `login:password`)
3. Интерактивный ввод

Логин берётся из `SCP_LOGIN`, `~/.config/ngasak/config.toml` (`scp.login`) или из keyring.

## Использование

### Полный автозапуск

```bash
ngasak run
```

Выполнит: проверка/подключение Ngate → запуск фильтрованного Sakura-autopush → ожидание `Online` → пауза `post_delay` → получение `compliance`/`info` от агента → `POST /api/login-er`.

### Управление Ngate

```bash
ngasak ngate status
ngasak ngate connect              # с автоподтверждением пуша по логину
ngasak ngate connect --all-pushes # подтверждать все пуши
ngasak ngate connect --no-autopush
ngasak ngate connect-gui          # через xdotool
```

### Управление Sakura 2FA

```bash
# Регистрация устройства по QR/deep-link
ngasak sakura register --qr "sakura://register?servername=...&serveraddr=...&servertoken=...&registrationtoken=..."
ngasak sakura register --server-url https://.../ --server-uuid <uuid>

# Опрос/подтверждение
ngasak sakura ping
ngasak sakura check
ngasak sakura approve <guid>
ngasak sakura deny <guid>
ngasak sakura history
ngasak sakura poll --interval 5

# Автоподтверждение
ngasak sakura autopush           # по логину из конфига/env
ngasak sakura autopush --all     # подтверждать все

# Локальный агент
ngasak sakura agent
```

### Только логин на scp.mos.ru

```bash
ngasak login-er
ngasak login-er --skip-ngate-check
```

### Конфиг

```bash
ngasak config show
ngasak config path
ngasak config init
```

## Особенности реализации

- **D-Bus меню Ngate**: парсинг через `busctl` (надёжнее zbus property-cache). При застревании в `AwaitingCredentials` автоматически делает сброс «Отключить» → «Подключить».
- **Sakura push-фильтр**: подтверждает только пуши с упоминанием нужного логина (`...под именем <login>`). Остальные игнорируются.
- **TLS для scp.mos.ru**: встроенный промежуточный сертификат GlobalSign GCC R6 AlphaSSL CA 2025. При редких 5xx делает повторы с обновлёнными `compliance`/`info`.
- **Совместимость**: полностью покрывает функционал `sakura2fa.py`, `sakura_autopush.py`, `ngate_connect_dbus.sh`, `scp_login_auto.sh`, `get_scp_creds.sh`.

## Файлы

- `~/.config/ngasak/config.toml` — конфиг NgaSak
- `~/.config/sakura2fa/config.json` — конфиг/токены Sakura (создаётся/обновляется при `sakura register`)
- `~/.local/share/NGateClient/` — логи Ngate

## Миграция sakura2fa -> ngasak

Теперь можно держать всё в одном конфиге `~/.config/ngasak/config.toml`. Старая копия сохранена в `~/.config/sakura2fa/config.json.bak` при миграции.

Текущий конфиг уже использует `sakura2fa/config.json`, но после миграции NgaSak будет читать/писать из `~/.config/ngasak/config.toml` в секции `[sakura]`.
