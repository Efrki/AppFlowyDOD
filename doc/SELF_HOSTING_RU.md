# Развёртывание AppFlowy для команды на своём VPS

Свой сервер синхронизации (AppFlowy-Cloud) с HTTPS, закрытой регистрацией и без AI-сервисов.
Данные команды хранятся только на вашем сервере.

Исходные данные, по которым написан гайд:

| Параметр | Значение |
|---|---|
| IP сервера | `90.156.128.128` |
| Домен | `tasks.ruchatting.ru` |
| ОС | Ubuntu (Debian-совместимая) |

## 0. Очистка сервера от старых сервисов

На сервере стоял Matrix-чат (`ruchatting.ru`), LiveKit-звонки и TeamSpeak. Раздел выполняется
только если решено их снести — **это необратимо, вся переписка пользователей будет потеряна**.

Сначала бэкап — на случай, если что-то из этого ещё понадобится:

```bash
mkdir -p /root/old-services-backup
tar czf /root/old-services-backup/matrix.tar.gz /etc/matrix-synapse /var/lib/matrix-synapse 2>/dev/null
tar czf /root/old-services-backup/livekit.tar.gz /etc/livekit /opt/lk-jwt-service 2>/dev/null
tar czf /root/old-services-backup/nginx.tar.gz /etc/nginx 2>/dev/null
ls -lh /root/old-services-backup/
```

Скачайте архивы к себе (`scp root@90.156.128.128:/root/old-services-backup/* .`) — и только потом удаляйте.

```bash
# остановить и отключить автозапуск
systemctl disable --now matrix-synapse livekit lk-jwt-service nginx

# удалить пакеты и данные
apt purge -y matrix-synapse-py3 nginx nginx-common
rm -rf /etc/matrix-synapse /var/lib/matrix-synapse
rm -rf /etc/livekit /opt/lk-jwt-service /usr/local/bin/livekit-server
rm -f /etc/systemd/system/livekit.service /etc/systemd/system/lk-jwt-service.service
rm -rf /etc/nginx
systemctl daemon-reload

# пользователь TeamSpeak, если сервис больше не нужен
userdel -r teamspeak 2>/dev/null

# старые сертификаты Let's Encrypt
certbot delete --cert-name livekit.ruchatting.ru 2>/dev/null
certbot delete --cert-name ruchatting.ru 2>/dev/null
```

Проверка, что порты 80/443 освободились:

```bash
ss -tlnp | grep -E ':80|:443'   # вывод должен быть пустым
```

## 0b. Если нужно переустановить сам AppFlowy с нуля

Удаляет контейнеры и все данные AppFlowy-Cloud:

```bash
cd ~/AppFlowy-Cloud 2>/dev/null && docker compose down -v
cd ~ && rm -rf ~/AppFlowy-Cloud
certbot delete --cert-name tasks.ruchatting.ru 2>/dev/null
```

## 1. Базовая защита сервера

**SSH по ключу.** Сейчас вход, скорее всего, по паролю — это первое, что брутфорсят. Порядок
важен: сначала добавить ключ и проверить, что он работает, и только потом отключать пароль.

На своём компьютере (не на сервере):

```bash
ssh-keygen -t ed25519            # если ключа ещё нет
ssh-copy-id root@90.156.128.128  # скопировать публичный ключ на сервер
```

**Не закрывая текущую сессию**, откройте второе окно терминала и проверьте вход: `ssh root@90.156.128.128`.
Зашло без пароля — можно отключать парольный вход на сервере:

```bash
sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
systemctl restart ssh
```

**Файрвол:**

```bash
apt install -y ufw
ufw default deny incoming
ufw allow ssh
ufw allow 80/tcp
ufw allow 443/tcp
ufw enable
```

Postgres и MinIO наружу не выставляются самим docker-compose — отдельно закрывать не нужно.

**Автообновления безопасности:**

```bash
apt install -y unattended-upgrades
dpkg-reconfigure -plow unattended-upgrades
```

## 2. DNS

В панели управления доменом `ruchatting.ru` добавьте A-запись:

| Тип | Имя | Значение |
|---|---|---|
| A | `tasks` | `90.156.128.128` |

Проверка (может занять от минуты до пары часов):

```bash
apt install -y dnsutils
dig +short tasks.ruchatting.ru    # должен вернуться 90.156.128.128
```

Не идите дальше, пока запись не разошлась — сертификат без неё не выпустится.

## 3. Установка AppFlowy-Cloud

```bash
# Docker, если его ещё нет
curl -fsSL https://get.docker.com | sh

git clone https://github.com/AppFlowy-IO/AppFlowy-Cloud.git
cd AppFlowy-Cloud
cp deploy.env .env
```

**Сгенерируйте секреты** — дефолты в `deploy.env` публично известны, оставлять их нельзя:

```bash
echo "JWT:      $(openssl rand -base64 48)"
echo "POSTGRES: $(openssl rand -base64 24)"
echo "MINIO:    $(openssl rand -base64 24)"
echo "ADMIN:    $(openssl rand -base64 24)"
```

Откройте `.env` (`nano .env`) и приведите к такому виду:

| Переменная | Значение |
|---|---|
| `FQDN` | `tasks.ruchatting.ru` |
| `SCHEME` | `https` |
| `WS_SCHEME` | `wss` |
| `GOTRUE_ADMIN_EMAIL` | ваша почта администратора |
| `GOTRUE_ADMIN_PASSWORD` | сгенерированный `ADMIN` |
| `GOTRUE_JWT_SECRET` | сгенерированный `JWT` |
| `POSTGRES_PASSWORD` | сгенерированный `POSTGRES` |
| `AWS_ACCESS_KEY` | любое своё имя вместо `minioadmin` |
| `AWS_SECRET` | сгенерированный `MINIO` |
| `GOTRUE_MAILER_AUTOCONFIRM` | `true` — регистрация без подтверждения почты (SMTP не нужен) |
| `GOTRUE_DISABLE_SIGNUP` | пока `false`, закроем на шаге 6 |
| `AI_OPENAI_API_KEY` | оставить пустым — AI отключён, данные не уходят в OpenAI |

Сохраните секреты в свой менеджер паролей — восстановить их из системы потом нельзя.

```bash
chmod 600 .env
```

## 4. TLS-сертификат

nginx AppFlowy-Cloud ждёт файлы по фиксированным путям `nginx/ssl/certificate.crt` и
`nginx/ssl/private_key.key`. Порт 80 сейчас свободен, поэтому берём сертификат самым простым способом:

```bash
apt install -y certbot
certbot certonly --standalone -d tasks.ruchatting.ru --agree-tos -m ваша@почта

cp /etc/letsencrypt/live/tasks.ruchatting.ru/fullchain.pem nginx/ssl/certificate.crt
cp /etc/letsencrypt/live/tasks.ruchatting.ru/privkey.pem  nginx/ssl/private_key.key
```

**Автопродление** (сертификат живёт 90 дней). Создайте `/etc/letsencrypt/renewal-hooks/deploy/appflowy.sh`:

```bash
cat > /etc/letsencrypt/renewal-hooks/deploy/appflowy.sh <<'EOF'
#!/bin/sh
cd /root/AppFlowy-Cloud
cp /etc/letsencrypt/live/tasks.ruchatting.ru/fullchain.pem nginx/ssl/certificate.crt
cp /etc/letsencrypt/live/tasks.ruchatting.ru/privkey.pem  nginx/ssl/private_key.key
docker compose restart nginx
EOF
chmod +x /etc/letsencrypt/renewal-hooks/deploy/appflowy.sh
```

Чтобы при продлении освобождался порт 80, допишите в `/etc/letsencrypt/cli.ini`:

```ini
pre-hook = cd /root/AppFlowy-Cloud && docker compose stop nginx
post-hook = cd /root/AppFlowy-Cloud && docker compose start nginx
```

Проверка: `certbot renew --dry-run`.

## 5. Запуск

```bash
cd ~/AppFlowy-Cloud
docker compose up -d
docker compose ps    # все сервисы должны стать healthy
```

Если какой-то сервис не поднялся: `docker compose logs --tail 50 <имя_сервиса>`.

Проверьте в браузере:

- `https://tasks.ruchatting.ru` — страница AppFlowy Cloud;
- `https://tasks.ruchatting.ru/console` — админ-консоль, вход по `GOTRUE_ADMIN_EMAIL` / `GOTRUE_ADMIN_PASSWORD`.

## 6. Онбординг команды и закрытие регистрации

1. Каждый ставит клиент, собранный из этого репозитория (ветка `develop`), и в приложении:
   **Settings → Cloud Settings → AppFlowy Cloud Self-hosted**, URL `https://tasks.ruchatting.ru` — регистрируется.
2. Когда все зарегистрировались, закройте регистрацию:

```bash
nano .env      # GOTRUE_DISABLE_SIGNUP=true
docker compose up -d
```

Дальше новые аккаунты создаются только приглашением через `/console`.

Чтобы никто случайно не залогинился в чужое облако, URL можно зашить в сборку клиента —
файл `.env` в `frontend/appflowy_flutter/`:

```
APPFLOWY_CLOUD_URL=https://tasks.ruchatting.ru
```

## 7. Бэкапы

```bash
apt install -y restic
restic init -r sftp:backup-host:/appflowy-backup     # репозиторий на другой машине

cd ~/AppFlowy-Cloud
docker compose exec -T postgres pg_dumpall -U postgres > /tmp/pg.sql
restic backup -r sftp:backup-host:/appflowy-backup /tmp/pg.sql ./minio_data
rm /tmp/pg.sql
```

Поставьте на расписание (cron или systemd timer). Пароль restic-репозитория храните **не на этом
сервере** — иначе при его компрометации бэкапы бесполезны.

## 8. Обновления

```bash
cd ~/AppFlowy-Cloud && git pull && docker compose pull && docker compose up -d
```

Клиент пересобирается из этого репозитория (ветка `develop`):

```bash
cd frontend
cargo make --profile development-mac-arm64 appflowy-core-dev
cd appflowy_flutter && flutter build macos
```

## 9. Что защищено, а что нет

Защищено: трафик (TLS), доступ посторонних (регистрация закрыта), локальные данные на компьютерах
команды (шифрование секрета через Argon2id + XChaCha20-Poly1305, секрет в системном хранилище ключей).

**Не защищено:** содержимое документов на самом сервере. Диск VPS не шифрован, и тот, у кого есть
физический доступ к машине (хостер, изъятие), прочитает базу. Полное закрытие — сквозное шифрование
(E2E), когда сервер хранит только шифротекст; это отдельный этап работы.

Что снижает риск уже сейчас:

- включить FileVault / BitLocker / LUKS на компьютерах команды;
- регистрировать аккаунты на нейтральные почты, не на корпоративные;
- шифрованные бэкапы (раздел 7) с ключом вне сервера.

## Приложение: закрыть сервер VPN-ом (опционально)

Убирает сервер из публичного интернета целиком — снаружи открыт только порт WireGuard, AppFlowy
доступен лишь тем, у кого есть ключ.

```bash
apt install -y wireguard
wg genkey | tee /etc/wireguard/server.key | wg pubkey > /etc/wireguard/server.pub
```

`/etc/wireguard/wg0.conf`:

```ini
[Interface]
Address = 10.8.0.1/24
ListenPort = 51820
PrivateKey = <содержимое server.key>

# по блоку на каждого члена команды
[Peer]
PublicKey = <публичный ключ клиента>
AllowedIPs = 10.8.0.2/32
```

```bash
systemctl enable --now wg-quick@wg0
ufw allow 51820/udp
ufw delete allow 80/tcp
ufw delete allow 443/tcp
ufw allow from 10.8.0.0/24 to any port 443 proto tcp
```

В `.env` меняется `FQDN` на внутренний адрес (`10.8.0.1`), сертификат тогда самоподписанный —
либо оставить домен и получать сертификат через DNS-проверку (`certbot --manual --preferred-challenges dns`).

Учтите: WireGuard-трафик распознаётся DPI и в РФ может блокироваться. Альтернатива с тем же
эффектом, но неотличимая от обычного HTTPS — Xray/Reality (3x-ui) на этом же сервере.
