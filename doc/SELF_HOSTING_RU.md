# Развёртывание AppFlowy для команды на своём VPS

Цель: собственный сервер синхронизации (AppFlowy-Cloud) с HTTPS, закрытой регистрацией и без AI-сервисов.
Данные команды хранятся только на вашем сервере.

**Важно про этот конкретный сервер:** на нём уже работает продакшн-сервис — Matrix-чат
(`ruchatting.ru`) и LiveKit-звонки, с реальными пользователями. Системный nginx держит порты 80/443
под эти сайты, и **его нельзя останавливать и нельзя трогать его конфиги** — вся установка ниже
специально сделана так, чтобы вообще не задевать существующий nginx: свой сертификат через
DNS-проверку (без порта 80), свой контейнер nginx на отдельном порту. Ни одна команда в этом файле
не останавливает и не перезапускает системный `nginx.service`.

## 0. Если нужно снести неудачную попытку и начать заново

Если уже запускали `docker compose up` и хотите начать с чистого листа — это удалит только
контейнеры/данные AppFlowy-Cloud, **системный nginx, Matrix и LiveKit не затрагиваются**:

```bash
cd ~/AppFlowy-Cloud 2>/dev/null && docker compose down -v   # -v: удалить и volume'ы (Postgres, MinIO)
cd ~
rm -rf ~/AppFlowy-Cloud

# сертификат AppFlowy, если успели получить (подставьте свой домен)
certbot delete --cert-name tasks.example.com 2>/dev/null

# на всякий случай убедиться, что ничего лишнего не осталось
docker ps -a          # не должно быть контейнеров appflowy/gotrue/minio/postgres-от-appflowy
docker volume ls | grep appflowy
```

После этого можно заново идти по шагам с раздела 1.

## Требования

- Домен или поддомен, A-запись которого указывает на IP этого сервера
  (например `tasks.ruchatting.ru`, если удобно использовать поддомен существующего домена).
- Доступ к панели DNS этого домена (для получения сертификата без порта 80).
- Docker + docker compose plugin (скорее всего уже стоит, раз крутится Matrix).

Ниже везде вместо `tasks.example.com` подставляйте свой домен, вместо `8443` — выбранный вами порт
(любой свободный, не занятый Matrix/LiveKit; проверить: `ss -tlnp | grep :8443`).

## 1. Установка AppFlowy-Cloud

```bash
git clone https://github.com/AppFlowy-IO/AppFlowy-Cloud.git
cd AppFlowy-Cloud
cp deploy.env .env
```

Обязательно поменяйте в `.env`:

| Переменная | Что поставить |
|---|---|
| `FQDN` | `tasks.example.com` |
| `SCHEME` | `https` |
| `WS_SCHEME` | `wss` |
| `NGINX_PORT` | `8080` (или другой свободный — под HTTP, используется только для собственных нужд контейнера) |
| `NGINX_TLS_PORT` | `8443` (или другой свободный — основной порт, на него будут заходить клиенты) |
| `GOTRUE_ADMIN_EMAIL` / `GOTRUE_ADMIN_PASSWORD` | свои админ-креды, пароль 20+ символов |
| `GOTRUE_JWT_SECRET` | результат `openssl rand -base64 48` |
| `POSTGRES_PASSWORD` | результат `openssl rand -base64 24` |
| `AWS_ACCESS_KEY` / `AWS_SECRET` | свои значения вместо `minioadmin`/`minioadmin` |
| `GOTRUE_MAILER_AUTOCONFIRM` | `true` — регистрация без подтверждения почты (SMTP не нужен); после онбординга команды регистрация всё равно закрывается, см. шаг 4 |
| `GOTRUE_DISABLE_SIGNUP` | пока `false`, поменяем на `true` на шаге 4 |
| `AI_OPENAI_API_KEY` | оставить пустым — AI-функции отключены, данные не уходят в OpenAI |

## 2. TLS-сертификат — без порта 80, через DNS

nginx AppFlowy-Cloud ждёт файлы по путям `./nginx/ssl/certificate.crt` и `./nginx/ssl/private_key.key`.
Получаем сертификат Let's Encrypt через DNS-проверку (TXT-запись), это не требует порта 80 и не
взаимодействует с системным nginx вообще:

```bash
certbot certonly --manual --preferred-challenges dns \
  -d tasks.example.com --agree-tos --email you@example.com
```

Certbot попросит добавить TXT-запись вида `_acme-challenge.tasks.example.com` в панели DNS вашего
домена — добавляете, ждёте минуту-две, подтверждаете в терминале Enter'ом.

```bash
cp /etc/letsencrypt/live/tasks.example.com/fullchain.pem nginx/ssl/certificate.crt
cp /etc/letsencrypt/live/tasks.example.com/privkey.pem  nginx/ssl/private_key.key
```

**Про продление:** в `--manual` режиме certbot не продлевает сертификат автоматически — раз в
~80 дней придётся повторить команду выше и снова вписать TXT-запись (Let's Encrypt пришлёт письмо
за пару недель до истечения). Если ваш DNS-провайдер поддерживается плагином certbot
(Cloudflare, Route53, DigitalOcean, Yandex Cloud и др.) — можно настроить полностью автоматическое
продление через API; скажите, у кого зарегистрирован домен, и это можно донастроить отдельно.

## 3. Запуск

```bash
docker compose up -d
docker compose ps   # все сервисы должны стать healthy
```

Firewall — открываем только новый порт, 80/443 уже открыты под Matrix и не трогаются:

```bash
ufw allow 8443/tcp
```

Проверьте в браузере: `https://tasks.example.com:8443` — должна открыться страница AppFlowy Cloud,
`https://tasks.example.com:8443/console` — админ-консоль (вход по `GOTRUE_ADMIN_EMAIL`/`PASSWORD`).

## 4. Онбординг команды и закрытие регистрации

1. Каждый член команды ставит собранный из этого репозитория клиент (ветка `develop`),
   в приложении: **Settings → Cloud Settings → AppFlowy Cloud Self-hosted**,
   URL: `https://tasks.example.com:8443` — и регистрируется.
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
APPFLOWY_CLOUD_URL=https://tasks.example.com:8443
```

## 5. Данные на диске и бэкапы

- На компьютерах команды включить FileVault (macOS) / BitLocker (Windows) / LUKS (Linux) —
  локальная база AppFlowy лежит на диске в открытом виде, дисковое шифрование это закрывает.
- Диск VPS не шифрован, и содержимое документов на сервере читаемо для того, у кого есть физический
  доступ к машине (хостер, изъятие). Это осознанное ограничение текущей схемы; полное закрытие —
  сквозное шифрование (E2E), отдельный этап. Минимум сейчас — шифрованные бэкапы наружу
  и регистрация команды на нейтральные почты.
- Бэкапьте только каталог AppFlowy-Cloud — общий сервер с Matrix, не задевайте его данные (обычно
  `/var/lib/matrix-synapse` и media store LiveKit) в этих же скриптах, чтобы не перепутать восстановление.

```bash
# Postgres + MinIO, шифрование средствами restic
restic init -r sftp:backup-host:/appflowy-backup
cd ~/AppFlowy-Cloud
docker compose exec postgres pg_dumpall -U postgres > /tmp/pg.sql
restic backup /tmp/pg.sql ./minio_data
```

Бэкапы — по расписанию (cron/systemd timer), ключ restic хранить отдельно от сервера.

## 6. Обновления

```bash
cd ~/AppFlowy-Cloud && git pull && docker compose pull && docker compose up -d
```

Это затрагивает только контейнеры AppFlowy-Cloud, системный nginx/Matrix/LiveKit не перезапускаются.

Клиент пересобирается из этого репозитория (ветка `develop`) командой
`cargo make --profile <ваш профиль, напр. development-mac-arm64> appflowy-core-dev`
и далее `flutter run`/`flutter build` в `frontend/appflowy_flutter`.

## Приложение: закрытие контура через VPN (опционально, на будущее)

Если позже захотите убрать сервер из публичного интернета целиком, схема такая: WireGuard-сеть
`10.8.0.0/24`, свой отдельный порт под него, доступ к `8443` только из VPN, у каждого члена команды —
свой ключ. На этом сервере уже есть открытые публичные сервисы (Matrix/LiveKit), поэтому полностью
закрыть его в VPN не получится — актуально только для порта самого AppFlowy. Учтите, что WireGuard-
трафик распознаётся DPI и в РФ может блокироваться; рабочая альтернатива — Xray/Reality (3x-ui) на
этом же сервере, трафик к AppFlowy маршрутизируется через него и неотличим от обычного HTTPS.

```bash
apt install wireguard
wg genkey | tee /etc/wireguard/server.key | wg pubkey > /etc/wireguard/server.pub
```

`/etc/wireguard/wg0.conf` (сервер):

```ini
[Interface]
Address = 10.8.0.1/24
ListenPort = 51821
PrivateKey = <содержимое server.key>

# по одному блоку на каждого члена команды
[Peer]
PublicKey = <публичный ключ клиента>
AllowedIPs = 10.8.0.2/32
```

Файрвол при этом: `ufw allow 51821/udp`, а `8443/tcp` вместо публичного открытия — закрываем и
разрешаем только с `10.8.0.0/24` (через `ufw route`/`iptables`, отдельная настройка под ваш случай).
