# hss — SSH Manager: Design Spec

Date: 2026-06-16

## Overview

`hss` — минималистичный TUI-менеджер SSH-серверов, вдохновлённый интерфейсом Claude Code CLI. Написан на Rust. Источник данных о хостах — Ansible inventory (INI-формат, только чтение). Все дополнительные данные (учётные данные, последний выбор, настройки) хранятся в приложении.

## CLI-режимы

```
hss                # TUI режим (основной)
hss --fzf          # Quick fzf режим — inline picker
hss <host|ip>      # Прямое подключение по имени или IP
```

## Архитектура

Один бинарный крейт (`hss`). Модули внутри крейта:

- `inventory` — парсинг Ansible INI, sync при старте
- `credentials` — CRUD, взаимодействие с keychain и TOML
- `ssh` — spawn ssh через PTY, SSH_ASKPASS helper
- `tui` — ratatui-экраны и event loop
- `fzf` — skim-based quick mode
- `config` — чтение/запись config/servers/credentials TOML
- `app` — точка входа, роутинг режимов

### Стек зависимостей

| Назначение | Crate |
|---|---|
| TUI | `ratatui` + `crossterm` |
| PTY для ssh | `portable-pty` |
| OS Keychain | `keyring` |
| Сериализация | `serde` + `toml` |
| fzf picker | `skim` |
| UUID | `uuid` |
| Парсинг INI | `configparser` или ручной парсер |

## Данные

### Файловая структура

```
~/.config/hss/
  config.toml        # путь до inventory, default_credential_id
  credentials.toml   # метаданные credentials (без паролей)
  servers.toml       # маппинг хост → last_credential_id
```

### config.toml

```toml
inventory_path = "/home/user/ansible/inventory.ini"
default_credential_id = "uuid-1"
```

### credentials.toml

```toml
[[credential]]
id = "uuid-1"
name = "deploy key"
username = "deploy"
kind = "key"           # "key" | "password"
key_path = "/home/user/.ssh/id_rsa"  # только для kind = "key"
# пароли хранятся в OS Keychain под ключом "hss:<id>"
```

### servers.toml

```toml
[[server]]
name = "web1"
last_credential_id = "uuid-1"
```

### Модель хоста (из inventory)

```rust
struct Host {
    name: String,
    ip: String,
    group: String,
    port: u16,                     // ansible_port или 22
    ansible_user: Option<String>,  // ansible_user если задан
}
```

### Sync с inventory при старте

1. Если `inventory_path` не задан (первый запуск) → открываем экран Settings, поле пути активно, ждём ввода перед продолжением
2. Парсим inventory → получаем актуальный список хостов
3. Читаем `servers.toml`
4. Удаляем из `servers.toml` записи хостов, которых нет в inventory

## TUI-экраны

### Главный экран

```
hss · ssh manager · N hosts
┌─────────────────────────────────────────────────────┐
│ 🔍 <поисковая строка — активна по умолчанию>        │
└─────────────────────────────────────────────────────┘
NAME             GROUP        HOST            PORT   LAST CONN
▶ web1           webservers   192.168.1.10    2222   2h ago
  web2           webservers   192.168.1.11    22     1d ago
  db1            databases    10.0.0.5        22     —

Enter=connect  C=credentials  S=settings  R=switch creds  Q=quit
```

- Поиск фильтрует по name, group, host (fuzzy)
- Стрелки / j/k для навигации по таблице
- Поиск активен сразу при открытии; Tab переключает фокус поиск↔таблица

### Popup выбора credentials

Появляется поверх главного экрана при подключении без сохранённого выбора или при нажатии `R`:

```
Выберите учётные данные для web1:
▶ deploy key    key       deploy
  admin pass    password  admin
  root key      key       root

Enter=подключиться  Esc=отмена
```

### Экран Credentials (`C`)

```
NAME             TYPE      USERNAME   DEFAULT
▶ deploy key     key       deploy     ★ default
  admin pass     password  admin
  root key       key       root

A=добавить  E=изменить  D=удалить  *=set default  Esc=назад
```

### Форма добавления/редактирования credential

Inline-форма поверх списка credentials:

```
Тип:  [● Password]  [ Key ]

Название:   [admin pass         ]
Логин:      [admin              ]
Пароль:     [••••••••           ]   ← скрыт при вводе

Tab=след.поле  Enter=сохранить  Esc=отмена
```

Для типа `key` вместо поля пароля — поле пути к файлу ключа.

### Экран Settings (`S`)

```
ANSIBLE INVENTORY PATH
[/home/user/ansible/inventory.ini    ]
Источник данных о хостах. Только чтение.

DEFAULT CREDENTIAL
★ deploy key (deploy) — key

Enter=сохранить  Esc=назад
```

## SSH Connection Flow

### TUI и прямой режим

```
1. Пользователь нажимает Enter (или hss <host>)
2. Есть last_credential_id для хоста?
   → Да: пробуем подключиться с этим credential
       → Успех: ssh занимает терминал
       → Ошибка auth: показываем popup выбора credentials
   → Нет: смотрим default_credential_id из config.toml
       → Если задан default_credential → подключаемся сразу с ним
       → Если не задан и credential только один → используем его
       → Иначе → popup выбора credentials
3. После успешного подключения: записываем credential_id в servers.toml
4. R на сервере → всегда показываем popup (принудительная смена)
```

### Механизм SSH_ASKPASS (передача пароля)

```
1. Записываем пароль в env-переменную HSS_PASSWORD
2. Создаём /tmp/hss-askpass-<pid> — shell-скрипт: `echo "$HSS_PASSWORD"`
3. chmod 700 /tmp/hss-askpass-<pid>
4. Spawn: ssh с env SSH_ASKPASS=/tmp/hss-askpass-<pid>, SSH_ASKPASS_REQUIRE=force
5. После завершения ssh: удаляем /tmp/hss-askpass-<pid>, очищаем env
```

Пароль не попадает в историю shell, не виден в `ps aux`.

### Quick fzf режим (`hss --fzf`)

```
1. Запускаем skim picker по списку хостов (name · group · host:port)
2. Enter → тот же connection flow что в TUI
3. Если нужен выбор credentials → второй skim picker
```

### Прямое подключение (`hss <host|ip>`)

```
hss web1          # ищем в inventory по имени → connection flow
hss 192.168.1.10  # ищем по IP → если нет в inventory, подключаемся напрямую
                  # для хостов не из inventory last_credential не сохраняется
```

## Error Handling

- Inventory-файл не найден → ошибка при старте с понятным сообщением и путём
- Keychain недоступен → предупреждение, предложение ввести пароль вручную через PTY
- SSH завершился с ошибкой → показываем exit code, предлагаем повторить с другими credentials

## Нереализованное (YAGNI)

- Туннели / port forwarding — можно добавить позже через доп. параметры ssh
- Группировка в TUI (collapse/expand) — поиск по группе покрывает потребность
- Несколько inventory-файлов — один файл покрывает заявленный use-case
- ProxyJump UI — передаётся через ~/.ssh/config, hss не трогает
