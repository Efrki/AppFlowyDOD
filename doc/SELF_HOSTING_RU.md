# Развёртывание AppFlowy для команды на своём VPS

Цель: собственный сервер синхронизации (AppFlowy-Cloud) с HTTPS, закрытой регистрацией и без AI-сервисов.
Данные команды хранятся только на вашем сервере.

## Требования

- VPS: 2+ vCPU, 4+ ГБ RAM, 40+ ГБ диска (основной рост — файлы-вложения в MinIO), Ubuntu 22.04/24.04.
- Docker + docker compose plugin.
- Домен, A-запись которого указывает на IP сервера (нужен для TLS-сертификата Let's Encrypt).
  Подойдёт любой дешёвый домен или поддомен.

Ниже везде вместо `tasks.example.com` подставляйте свой домен.

## 1. Базовая гигиена сервера

```bash
# вход только по SSH-ключу
sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
systemctl restart ssh

# файрвол: наружу только SSH и веб
ufw default deny incoming
ufw allow ssh
ufw allow 80/tcp
ufw allow 443/tcp
ufw enable
```

Postgres и MinIO наружу не выставляются самим docker-compose — дополнительно закрывать их не нужно.

## 2. Установка AppFlowy-Cloud

```bash
git clone https://github.com/AppFlowy-IO/AppFlowy-Cloud.git
cd AppFlowy-Cloud
cp deploy.env .env
```

Обязательно поменяйте в `.env` (дефолты небезопасны):

| Переменная | Что поставить |
|---|---|
| `FQDN` | `tasks.example.com` |
| `SCHEME` | `https` |
| `WS_SCHEME` | `wss` |
| `GOTRUE_ADMIN_EMAIL` / `GOTRUE_ADMIN_PASSWORD` | свои админ-креды, пароль 20+ символов |
| `GOTRUE_JWT_SECRET` | результат `openssl rand -base64 48` |
| `POSTGRES_PASSWORD` | результат `openssl rand -base64 24` |
| `AWS_ACCESS_KEY` / `AWS_SECRET` | свои значения вместо `minioadmin`/`minioadmin` |
| `GOTRUE_MAILER_AUTOCONFIRM` | `true` — регистрация без подтверждения почты (SMTP не нужен); после онбординга команды регистрация всё равно закрывается, см. шаг 5 |
| `GOTRUE_DISABLE_SIGNUP` | пока `false`, поменяем на `true` на шаге 5 |
| `AI_OPENAI_API_KEY` | оставить пустым — AI-функции отключены, данные не уходят в OpenAI |

## 3. TLS-сертификат

nginx из docker-compose ждёт файлы по фиксированным путям
`./nginx/ssl/certificate.crt` и `./nginx/ssl/private_key.key`. Получаем их через Let's Encrypt:

```bash
apt install certbot
# порт 80 должен быть свободен (компоуз ещё не запущен)
certbot certonly --standalone -d tasks.example.com

cp /etc/letsencrypt/live/tasks.example.com/fullchain.pem nginx/ssl/certificate.crt
cp /etc/letsencrypt/live/tasks.example.com/privkey.pem  nginx/ssl/private_key.key
```

Автопродление (сертификат живёт 90 дней) — создайте `/etc/letsencrypt/renewal-hooks/deploy/appflowy.sh`:

```bash
#!/bin/sh
cd /root/AppFlowy-Cloud
cp /etc/letsencrypt/live/tasks.example.com/fullchain.pem nginx/ssl/certificate.crt
cp /etc/letsencrypt/live/tasks.example.com/privkey.pem  nginx/ssl/private_key.key
docker compose restart nginx
```

```bash
chmod +x /etc/letsencrypt/renewal-hooks/deploy/appflowy.sh
```

Certbot сам продлевает по таймеру; при продлении нужно, чтобы порт 80 был доступен ему —
добавьте в `/etc/letsencrypt/cli.ini` строки:

```ini
pre-hook = cd /root/AppFlowy-Cloud && docker compose stop nginx
post-hook = cd /root/AppFlowy-Cloud && docker compose start nginx
```

Проверка: `certbot renew --dry-run`.

## 4. Запуск

```bash
docker compose up -d
docker compose ps   # все сервисы должны стать healthy
```

Проверьте в браузере: `https://tasks.example.com` — должна открыться страница AppFlowy Cloud,
`https://tasks.example.com/console` — админ-консоль (вход по `GOTRUE_ADMIN_EMAIL`/`PASSWORD`).

## 5. Онбординг команды и закрытие регистрации

1. Каждый член команды ставит собранный из этого репозитория клиент (ветка `develop`),
   в приложении: **Settings → Cloud Settings → AppFlowy Cloud Self-hosted**, URL: `https://tasks.example.com` —
   и регистрируется.
2. Когда все зарегистрировались — закройте регистрацию насовсем:

```bash
# в .env
GOTRUE_DISABLE_SIGNUP=true
docker compose up -d
```

Новые аккаунты теперь создаются только приглашением через админ-консоль (`/console`).

Чтобы никто из команды случайно не залогинился в чужое облако, URL сервера можно зашить в сборку клиента —
файл `.env` в `frontend/appflowy_flutter/` при сборке:

```
APPFLOWY_CLOUD_URL=https://tasks.example.com
```

## 6. Данные на диске и бэкапы

- На компьютерах команды включить FileVault (macOS) / BitLocker (Windows) / LUKS (Linux) —
  локальная база AppFlowy лежит на диске в открытом виде, дисковое шифрование это закрывает.
- Диск VPS не шифрован, и содержимое документов на сервере читаемо для того, у кого есть физический
  доступ к машине (хостер, изъятие). Это осознанное ограничение текущей схемы; полное закрытие —
  сквозное шифрование (E2E), отдельный этап. Минимум сейчас — шифрованные бэкапы наружу
  и регистрация команды на нейтральные почты.

```bash
# Postgres + MinIO, шифрование средствами restic
restic init -r sftp:backup-host:/appflowy-backup
docker compose exec postgres pg_dumpall -U postgres > /tmp/pg.sql
restic backup /tmp/pg.sql ./minio_data
```

Бэкапы — по расписанию (cron/systemd timer), ключ restic хранить отдельно от сервера.

## 7. Обновления

```bash
cd AppFlowy-Cloud && git pull && docker compose pull && docker compose up -d
```

Клиент пересобирается из этого репозитория (ветка `develop`) командой
`cargo make --profile <ваш профиль, напр. development-mac-arm64> appflowy-core-dev`
и далее `flutter run`/`flutter build` в `frontend/appflowy_flutter`.

## Приложение: закрытие контура через VPN (опционально, на будущее)

Если позже захотите убрать сервер из публичного интернета целиком, схема такая: WireGuard-сеть
`10.8.0.0/24`, наружу открыты только SSH и `51820/udp`, порты 80/443 доступны только из VPN,
у каждого члена команды — свой ключ. Учтите, что WireGuard-трафик распознаётся DPI и в РФ может
блокироваться; рабочая альтернатива — Xray/Reality (3x-ui) на этом же сервере, трафик к AppFlowy
маршрутизируется через него и неотличим от обычного HTTPS.

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

Файрвол при этом меняется на: наружу только SSH и `51820/udp`, а 80/443 — только с `10.8.0.0/24`.
