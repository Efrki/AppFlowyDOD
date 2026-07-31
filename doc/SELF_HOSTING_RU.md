# Развёртывание AppFlowy для команды на своём VPS

Цель: собственный сервер синхронизации (AppFlowy-Cloud), закрытый от интернета, без AI-сервисов.
Данные команды не покидают ваш контур.

## Требования

- VPS: 2+ vCPU, 4+ ГБ RAM, 40+ ГБ диска, Ubuntu 22.04/24.04.
- Docker + docker compose plugin.
- Домен не обязателен, если доступ будет только через VPN (рекомендуемый вариант).

## 1. Установка сервера

```bash
git clone https://github.com/AppFlowy-IO/AppFlowy-Cloud.git
cd AppFlowy-Cloud
cp deploy.env .env
```

Обязательно поменяйте в `.env` (дефолты небезопасны):

| Переменная | Что поставить |
|---|---|
| `FQDN` | IP или домен сервера (для VPN — внутренний IP, напр. `10.8.0.1`) |
| `SCHEME` / `WS_SCHEME` | `https` / `wss` при TLS, иначе `http` / `ws` (только внутри VPN) |
| `GOTRUE_ADMIN_EMAIL` / `GOTRUE_ADMIN_PASSWORD` | свои админ-креды, пароль 20+ символов |
| `GOTRUE_JWT_SECRET` | `openssl rand -base64 48` |
| `POSTGRES_PASSWORD` | `openssl rand -base64 24` |
| `AWS_ACCESS_KEY` / `AWS_SECRET` | свои значения вместо `minioadmin`/`minioadmin` |
| `GOTRUE_MAILER_AUTOCONFIRM` | `true`, если SMTP не настраиваете (регистрация без подтверждения почты — ок внутри VPN) |
| `AI_OPENAI_API_KEY` | оставить пустым — AI-функции отключены, данные не уходят в OpenAI |

Запуск:

```bash
docker compose up -d
docker compose ps   # все сервисы должны быть healthy
```

Сервис `ai` можно дополнительно закомментировать в `docker-compose.yml` — без API-ключа он и так не активен, но так он даже не поднимется.

## 2. Закрытие контура (WireGuard)

Сервис не должен смотреть в интернет. На сервере:

```bash
apt install wireguard
wg genkey | tee /etc/wireguard/server.key | wg pubkey > /etc/wireguard/server.pub
```

`/etc/wireguard/wg0.conf` (сервер):

```ini
[Interface]
Address = 10.8.0.1/24
ListenPort = 51820
PrivateKey = <содержимое server.key>

# по одному блоку на каждого члена команды
[Peer]
PublicKey = <публичный ключ клиента>
AllowedIPs = 10.8.0.2/32
```

Файрвол — наружу открыт только SSH и WireGuard:

```bash
ufw default deny incoming
ufw allow ssh
ufw allow 51820/udp
ufw enable
```

Порты AppFlowy-Cloud (80/443 nginx) при этом доступны только с адресов `10.8.0.0/24` через wg0.
У каждого члена команды — клиент WireGuard с маршрутом `10.8.0.0/24` через сервер.

Если VPN не подходит (мобильные без постоянного VPN и т.п.) — тогда публичный домен,
TLS через Let's Encrypt (`SCHEME=https`, `WS_SCHEME=wss`) и обязательно сильные пароли + отключённая
регистрация посторонних (SMTP-подтверждение вместо `GOTRUE_MAILER_AUTOCONFIRM=true`).

## 3. Подключение клиентов

В настольном AppFlowy: **Settings → Cloud Settings → AppFlowy Cloud Self-hosted**, URL: `http://10.8.0.1` (или ваш домен).

Либо зашить URL в сборку, чтобы никто случайно не залогинился в чужое облако — файл `.env` рядом с бинарём при сборке фронтенда:

```
APPFLOWY_CLOUD_URL=http://10.8.0.1
```

## 4. Данные на диске и бэкапы

- На клиентах включить FileVault (macOS) / BitLocker (Windows) / LUKS (Linux) — локальная база AppFlowy лежит на диске в открытом виде, дисковое шифрование это закрывает.
- На VPS полноценный LUKS обычно недоступен; минимум — шифрованные бэкапы наружу:

```bash
# Postgres + MinIO, шифрование средствами restic
restic init -r sftp:backup-host:/appflowy-backup
docker compose exec postgres pg_dumpall -U postgres > /tmp/pg.sql
restic backup /tmp/pg.sql ./minio_data
```

Бэкапы — по расписанию (cron/systemd timer), ключ restic хранить отдельно от сервера.

## 5. Обновления

```bash
cd AppFlowy-Cloud && git pull && docker compose pull && docker compose up -d
```

Клиент собирается из этого репозитория (ветка `develop`) вашим обычным пайплайном сборки AppFlowy.
