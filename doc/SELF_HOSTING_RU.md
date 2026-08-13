# Развёртывание AppFlowy для команды на своём VPS

Свой сервер синхронизации (AppFlowy-Cloud) с HTTPS, закрытой регистрацией и без AI-сервисов.
Данные команды хранятся только на вашем сервере.

Исходные данные, по которым написан гайд:

| Параметр | Значение |
|---|---|
| IP сервера | `90.156.128.128` |
| Домен | `tasks.ruchatting.ru` |
| ОС | Ubuntu (Debian-совместимая) |

## Что на сервере нельзя трогать

На этой машине продолжает работать **TeamSpeak в Docker-контейнере**. Ни одна команда из гайда его
не задевает, но при ручных экспериментах помните:

| Объект | Значение |
|---|---|
| Контейнер | `teamspeak-server` |
| Том с данными | `teamspeak6_ts-data` |
| Конфиг | `/home/teamspeak/teamspeak6/docker-compose.yml` |
| Порты | UDP 9987, TCP 30033 |

**Никогда не выполняйте `docker system prune -a --volumes`** — эта команда снесёт том TeamSpeak
вместе с настройками сервера, каналами и правами. Для очистки AppFlowy пользуйтесь только
`docker compose down` из его собственной папки.

## 0. Очистка сервера от старых сервисов

Выполняется один раз. Matrix-чат (`ruchatting.ru`) и LiveKit сносим, TeamSpeak остаётся.
Снос Matrix **необратим — вся переписка пользователей будет потеряна**, поэтому сначала бэкап:

```bash
mkdir -p /root/old-services-backup
tar czf /root/old-services-backup/matrix.tar.gz /etc/matrix-synapse /var/lib/matrix-synapse /opt/matrix 2>/dev/null
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

# старые сертификаты Let's Encrypt
certbot delete --cert-name livekit.ruchatting.ru
certbot delete --cert-name ruchatting.ru
```

Matrix был установлен дважды — пакетом apt и контейнером Docker. Второй убираем отдельно
(имя контейнера подставьте из `docker ps -a`, у нас это `synapse`):

```bash
docker rm -f synapse
docker rmi matrixdotorg/synapse:latest
rm -rf /opt/matrix
apt autoremove -y          # снести зависимости, оставшиеся от Matrix (~сотни node-* пакетов)
```

Проверка — порты 80/443 свободны, TeamSpeak жив, лишних контейнеров нет:

```bash
ss -tlnp | grep -E ':80|:443'          # вывод должен быть пустым
ss -ulnp | grep 9987                   # TeamSpeak должен остаться в списке
docker ps -a                           # должен остаться только teamspeak-server
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
ufw status numbered
```

> **Важно: ufw не управляет портами Docker-контейнеров.** Docker пишет правила напрямую в iptables,
> в обход ufw. Поэтому порты TeamSpeak (9987/30033) и порты AppFlowy (80/443) доступны из интернета
> независимо от того, что показывает `ufw status` — открывать их правилами не нужно, но и закрыть
> ими не получится. ufw здесь защищает только не-контейнерные сервисы: SSH и всё, что вы поставите
> пакетами.
>
> Если понадобится ограничить доступ к самому AppFlowy (например, пускать только из VPN), это
> делается либо привязкой портов к localhost в его `docker-compose.yml`, либо правилами в цепочке
> `DOCKER-USER` — см. приложение про VPN.

Postgres и MinIO наружу не публикуются самим docker-compose, поэтому снаружи недоступны.

**Автообновления безопасности:**

```bash
apt install -y unattended-upgrades
dpkg-reconfigure -plow unattended-upgrades
```

## 2. DNS — уже настроен, ничего делать не нужно

У домена `ruchatting.ru` (NS: `nameself.com`) настроена wildcard-запись `*.ruchatting.ru`, поэтому
**любой** поддомен уже указывает на этот сервер. Доступ к панели регистратора не требуется.

Просто убедитесь, что имя резолвится:

```bash
apt install -y dnsutils
dig +short tasks.ruchatting.ru    # должен вернуться 90.156.128.128
```

Если вдруг вернулось пусто (wildcard убрали) — тогда нужна обычная A-запись `tasks` → `90.156.128.128`
в панели домена, и без неё сертификат не выпустится.

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

### Версии образов закреплены намеренно

Не убирайте эти строки из `.env` и не меняйте на `latest`:

```
APPFLOWY_CLOUD_VERSION=0.9.64
GOTRUE_VERSION=0.9.64
APPFLOWY_ADMIN_FRONTEND_VERSION=0.9.64
APPFLOWY_WORKER_VERSION=0.9.64
APPFLOWY_WEB_VERSION=0.1.19
```

Причина: `appflowyinc/appflowy_cloud:latest` собирается **не из открытых исходников**, а из
закрытого коммерческого форка, и бесплатный тариф в нём ограничен **одним пользователем на
рабочую область** (`Free plan limits - max_users: 1, max_guests: 3` в логах при старте).
Ограничение не документировано, ошибки не выдаёт: сервер молча отказывается создавать профиль,
`/api/user/verify` возвращает `is_new: false`, а потом всё падает с `User not found` — человек
просто не может войти.

`0.9.64` от 4 июля 2025 — последний образ, собранный из AGPL-исходников. Тем же днём датирован
последний релиз в открытом репозитории; всё, что выше по номеру (включая теги `0.9.1xx` от
декабря 2025), уже из форка, хотя нумерация продолжается та же.

### Заплатки в docker-compose.override.yml

Апстримный `docker-compose.yml` рассчитан на свежие образы, поэтому рядом лежит
`docker-compose.override.yml` — его тоже не удаляйте:

| Что чинит | Почему |
|---|---|
| `healthcheck` на `/health` | в 0.9.64 эндпоинт назывался так; `/api/health` появился позже, и штатная проверка валит по зависимости полстека |
| `ADMIN_FRONTEND_*` переменные | 0.9.64 читает их, свежий compose передаёт другие имена — консоль уходит на `localhost` и падает с паникой |
| `ADMIN_FRONTEND_PATH_PREFIX: /console` | без него консоль отдаёт ссылки на `/web` и уводит браузер в веб-клиент |
| `ai` в профиле `ai` | свежий образ AI требует миграцию, которой в 0.9.64 нет, и уходит в бесконечный перезапуск; AI не используется |

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

1. Соберите установщики и разошлите команде (см. §8 «Раздача клиента команде» — Windows качается
   готовым, macOS собирается локально).
2. Каждый ставит клиент и в приложении: **Settings → Cloud Settings → AppFlowy Cloud Self-hosted**,
   URL `https://tasks.ruchatting.ru` — регистрируется.
3. Когда все зарегистрировались, закройте регистрацию:

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

## 6b. Управление сервером: afctl

Вместо россыпи команд `docker`/`curl`/`psql` на сервере лежит `/root/afctl.sh`
(исходник — [`deploy/afctl.sh`](../deploy/afctl.sh)):

```bash
/root/afctl.sh status              # стек, лимиты плана, срок TLS, бэкапы — одним экраном
/root/afctl.sh users               # список учёток
/root/afctl.sh passwd <email>      # выдать пароль без письма (SMTP не нужен)
/root/afctl.sh up                  # обновить и дождаться готовности
/root/afctl.sh pin <версия>        # закрепить версию образа
/root/afctl.sh backup              # дамп БД + файлы minio + конфиги
/root/afctl.sh install-cron        # ежедневный бэкап в 04:30
```

Обновить сам скрипт:

```bash
curl -fsSL https://raw.githubusercontent.com/Efrki/AppFlowyDOD/develop/deploy/afctl.sh \
  -o /root/afctl.sh && chmod +x /root/afctl.sh
```

`passwd` — не роскошь, а необходимость: пока SMTP не настроен, кнопка «забыл пароль» отдаёт
500 (`Error sending recovery email`), и восстановить доступ человеку можно только так.

## 7. Бэкапы

Минимальный вариант уже включён — `afctl.sh install-cron` кладёт ежедневный архив в
`/var/backups/appflowy` (дамп postgres + том minio + `.env` + сертификаты, хранится 14 копий).
Архив содержит `.env` со всеми секретами, поэтому каталог держите закрытым и **увозите копию с
сервера**: сгоревший диск уносит и данные, и бэкап.

Более серьёзный вариант с дедупликацией и внешним хранилищем:

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

## 8. Обновления и раздача клиента команде

**Сервер:**

```bash
cd ~/AppFlowy-Cloud && git pull && docker compose pull && docker compose up -d
```

**Клиент — Windows.** Не заставляйте коллег ставить Rust/Flutter вручную — установщик собирает CI
(`.github/workflows/build_windows_dod.yml`) на серверах GitHub. Со своего Mac запушьте тег вида
`dod-<версия>` — именно на такую маску настроен триггер:

```bash
cd /Users/kirill/AppFlowyDOD
git checkout develop
git tag dod-1.0.0
git push origin dod-1.0.0
```

Версия не должна начинаться с буквы `v` и обязана начинаться с цифры. Через ~35 минут на
`https://github.com/Efrki/AppFlowyDOD/releases` появятся два файла:

- `AppFlowy-1.0.0-windows-x86_64.exe` — установщик, скачать и запустить;
- `AppFlowy-1.0.0-windows-x86_64.zip` — портативная версия, если установщик не нужен.

Windows SmartScreen покажет «неизвестный издатель» — это нормально для внутренней сборки без
подписи: «Подробнее → Всё равно выполнить».

На каждое обновление — новый тег с новым номером (`dod-1.0.1` и т.д.).

Три грабли, на которые уже наступили (не повторяйте):

| Симптом | Причина |
|---|---|
| Джоба вечно «Waiting for a runner» | образ `windows-2019` из upstream-воркфлоу выведен GitHub из эксплуатации |
| Сборка «успешна», но релиз пустой | на `windows-latest` стоит VS 2026, Flutter 3.27.4 её не знает и берёт генератор VS 2019 → CMake падает, а `duckscript exec` в `cargo make` глотает ошибку. Поэтому образ закреплён на `windows-2022`, а в воркфлоу есть шаг проверки наличия `AppFlowy.exe` |
| `.deb`/установщик ругается на версию | версия начиналась с `v` |

Upstream-воркфлоу `release.yml` в форке **отключён** намеренно: он падает на мёртвых раннерах и
требует секретов Docker Hub, которых у нас нет. Включать обратно не нужно.

**Клиент — macOS.** Собирается локально (CI для Mac требует платный Apple-сертификат подписи и
нотаризацию, которых у форка нет):

```bash
cd frontend
cargo make --profile development-mac-arm64 appflowy-dev
```

Цель именно `appflowy-dev`, а не `appflowy-core-dev`: последняя собирает только Rust-ядро, а Dart-код
(`LocaleKeys`, `FlowySvgs`, `*.freezed.dart`) создаётся отдельным шагом `code_generation`. Без него
`flutter build macos` падает с сотнями ошибок вида `The getter 'LocaleKeys' isn't defined`.

Требуется **полный Xcode** (не Command Line Tools) и CocoaPods:

```bash
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
sudo xcodebuild -runFirstLaunch
brew install cocoapods
flutter doctor        # строки Xcode и CocoaPods должны быть зелёными
```

Готовое приложение — `frontend/appflowy_flutter/product/<версия>/macos/Release/AppFlowy.app`.
Заzipуйте и передайте остальным Mac-пользователям; при первом запуске Gatekeeper тоже предупредит про
неизвестного разработчика — открывается через правый клик → "Открыть".

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
```

Закрыть публичный доступ к AppFlowy правилами ufw **не получится** — порты опубликованы Docker'ом
в обход ufw (см. врезку в разделе 1). Рабочий способ — привязать порты к VPN-адресу в
`docker-compose.yml` AppFlowy-Cloud:

```yaml
  nginx:
    ports:
      - "10.8.0.1:443:443"    # вместо "443:443"
```

После правки `docker compose up -d` — сервис станет доступен только внутри VPN.

В `.env` меняется `FQDN` на внутренний адрес (`10.8.0.1`), сертификат тогда самоподписанный —
либо оставить домен и получать сертификат через DNS-проверку (`certbot --manual --preferred-challenges dns`).

Учтите: WireGuard-трафик распознаётся DPI и в РФ может блокироваться. Альтернатива с тем же
эффектом, но неотличимая от обычного HTTPS — Xray/Reality (3x-ui) на этом же сервере.
