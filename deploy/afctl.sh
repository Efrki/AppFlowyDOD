#!/usr/bin/env bash
#
# afctl - управление self-hosted AppFlowy-Cloud одной командой.
#
# Ставится на сервер рядом со стеком. Секреты берутся из .env стека и никогда
# не попадают ни в этот файл, ни в репозиторий.
#
#   ./afctl.sh status              состояние стека, лимиты плана, число учёток
#   ./afctl.sh users               список пользователей
#   ./afctl.sh passwd <email>      задать пароль без письма, через admin API
#   ./afctl.sh up                  поднять/обновить стек и дождаться готовности
#   ./afctl.sh pin <версия>        закрепить версию образа appflowy_cloud
#   ./afctl.sh watch               следить за лимитами и приглашениями вживую
#   ./afctl.sh backup              дамп БД + файлов + конфигов
#   ./afctl.sh install-cron        ежедневный бэкап в 04:30
#   ./afctl.sh logs [сервис] [N]   хвост логов без мусора о переменных
#
set -euo pipefail

AF_DIR="${AF_DIR:-/root/AppFlowy-Cloud}"
BACKUP_DIR="${BACKUP_DIR:-/var/backups/appflowy}"
BACKUP_KEEP="${BACKUP_KEEP:-14}"

die()  { printf '\033[31mОШИБКА:\033[0m %s\n' "$*" >&2; exit 1; }
ok()   { printf '\033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*"; }
head_(){ printf '\n\033[1m== %s\033[0m\n' "$*"; }

# Справка - это шапка файла: печатаем комментарии со 2-й строки до первой
# строки кода, чтобы список команд не разъезжался при правках.
usage() { awk 'NR>1 && /^#/ {sub(/^# ?/,""); print; next} NR>1 {exit}' "$0"; }

# Справка не должна требовать ни стека, ни docker: её читают до установки.
case "${1:-}" in -h|--help|help) usage; exit 0 ;; esac

command -v docker >/dev/null || die "docker не установлен"
[ -d "$AF_DIR" ] || die "не найден каталог стека: $AF_DIR (меняется переменной AF_DIR)"
cd "$AF_DIR"
[ -f .env ] || die "не найден $AF_DIR/.env"

# docker compose предупреждает о каждой незаданной необязательной переменной,
# и полезный вывод тонет в этом шуме.
dc()  { docker compose "$@" 2>&1 | grep -v 'variable is not set' || true; }
dcq() { docker compose "$@" 2>/dev/null; }

# Значение из .env; второй аргумент - значение по умолчанию для пустых/отсутствующих.
envget() {
  local v
  v="$(sed -n "s/^${1}=//p" .env | tail -1 | sed 's/^"\(.*\)"$/\1/; s/^'"'"'\(.*\)'"'"'$/\1/')"
  printf '%s' "${v:-${2:-}}"
}

FQDN="$(envget FQDN)"
BASE="$(envget SCHEME https)://${FQDN}"
PGUSER="$(envget POSTGRES_USER postgres)"
PGDB="$(envget POSTGRES_DB postgres)"

json() { python3 -c 'import json,sys;print(json.dumps(dict(zip(sys.argv[1::2],sys.argv[2::2]))))' "$@"; }

# Админский токен gotrue: нужен, чтобы менять пароли и читать список учёток
# без отправки писем.
admin_token() {
  command -v curl >/dev/null || die "нужен curl"
  command -v python3 >/dev/null || die "нужен python3"
  local email pass
  email="$(envget GOTRUE_ADMIN_EMAIL)"
  pass="$(envget GOTRUE_ADMIN_PASSWORD)"
  [ -n "$email" ] && [ -n "$pass" ] || die "в .env нет GOTRUE_ADMIN_EMAIL/GOTRUE_ADMIN_PASSWORD"

  curl -sS -m 20 -X POST "$BASE/gotrue/token?grant_type=password" \
    -H 'Content-Type: application/json' \
    --data-binary "$(json email "$email" password "$pass")" \
  | python3 -c '
import json, sys
d = json.load(sys.stdin)
if "access_token" not in d:
    sys.exit("вход админом не удался: %s" % d.get("msg", d))
print(d["access_token"])
'
}

api() {
  local method="$1" path="$2"; shift 2
  curl -sS -m 30 -X "$method" "$BASE/gotrue$path" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' "$@"
}

# ---------------------------------------------------------------- команды ---

cmd_status() {
  head_ "Контейнеры"
  dcq ps --format '{{.Service}}|{{.Status}}' | sort | awk -F'|' '{printf "   %-18s %s\n",$1,$2}'

  head_ "Тариф и лицензия"
  local plan
  plan="$(dcq logs appflowy_cloud 2>/dev/null | grep -i 'Free plan limits' | tail -1 || true)"
  if [ -n "$plan" ]; then
    warn "образ с лицензионным ограничением"
    printf '   %s\n' "$(sed 's/.*"message":"//; s/".*//' <<<"$plan")"
    printf '   текущая версия: %s\n' "$(envget APPFLOWY_CLOUD_VERSION latest)"
    # 0.9.64 (4 июля 2025) - последний образ, собранный из открытых исходников:
    # этим же днём датирован последний релиз в AGPL-репозитории. Всё, что выше,
    # собрано из закрытого форка, включая теги 0.9.1xx с той же нумерацией.
    printf '   снять ограничение: %s pin 0.9.64\n' "$0"
  else
    ok "ограничений плана в логах нет"
  fi

  head_ "Учётные записи"
  local n
  n="$(dcq exec -T postgres psql -U "$PGUSER" -d "$PGDB" \
        -tAc 'SELECT count(*) FROM af_user WHERE deleted_at IS NULL' 2>/dev/null | tr -d ' ' || true)"
  printf '   активных пользователей: %s\n' "${n:-не удалось определить}"
  if [ "$(envget GOTRUE_DISABLE_SIGNUP false)" = "true" ]; then
    ok "регистрация закрыта"
  else
    warn "регистрация ОТКРЫТА - закрыть после онбординга команды"
  fi

  head_ "Почта"
  if dcq logs --tail 500 gotrue 2>/dev/null | grep -q 'Error sending'; then
    warn "SMTP не работает: сброс пароля и приглашения по почте недоступны"
    printf '   пароли выдавайте так: %s passwd <email>\n' "$0"
  else
    ok "ошибок отправки в свежих логах нет"
  fi

  head_ "TLS"
  if [ -f nginx/ssl/certificate.crt ]; then
    local until_ days
    until_="$(openssl x509 -enddate -noout -in nginx/ssl/certificate.crt | cut -d= -f2)"
    days=$(( ( $(date -d "$until_" +%s) - $(date +%s) ) / 86400 ))
    if [ "$days" -lt 14 ]; then warn "сертификат истекает через $days дн."
    else ok "сертификат действителен ещё $days дн."; fi
  else
    warn "нет nginx/ssl/certificate.crt"
  fi

  head_ "Бэкапы"
  local last
  last="$(ls -1t "$BACKUP_DIR"/appflowy-*.tar.gz 2>/dev/null | head -1 || true)"
  if [ -n "$last" ]; then
    ok "последний: $(basename "$last"), $(du -h "$last" | cut -f1)"
  else
    warn "бэкапов нет - включить: $0 install-cron"
  fi
  echo
}

cmd_users() {
  TOKEN="$(admin_token)"
  api GET '/admin/users?per_page=200' | python3 -c '
import json, sys
d = json.load(sys.stdin)
users = d.get("users", d if isinstance(d, list) else [])
if not users:
    sys.exit("список пуст или ответ не распознан: %s" % str(d)[:200])
w = max(len(u.get("email", "")) for u in users)
print("   %-*s  %-19s  %s" % (w, "EMAIL", "СОЗДАН", "ID"))
for u in sorted(users, key=lambda x: x.get("created_at", "")):
    print("   %-*s  %-19s  %s" % (w, u.get("email", ""), u.get("created_at", "")[:19], u.get("id", "")))
print("\n   всего: %d" % len(users))
'
}

cmd_passwd() {
  local email="${1:-}" pass="${2:-}" generated=0
  [ -n "$email" ] || die "укажите почту: $0 passwd user@example.com [пароль]"
  if [ -z "$pass" ]; then
    pass="$(openssl rand -base64 18 | tr -d '/+=' | cut -c1-20)"
    generated=1
  fi

  TOKEN="$(admin_token)"
  local uid
  uid="$(api GET '/admin/users?per_page=200' | python3 -c '
import json, sys
d = json.load(sys.stdin)
users = d.get("users", d if isinstance(d, list) else [])
hit = [u for u in users if u.get("email", "").lower() == sys.argv[1].lower()]
print(hit[0]["id"] if hit else "")
' "$email")"
  [ -n "$uid" ] || die "пользователь $email не найден. Список: $0 users"

  api PUT "/admin/users/$uid" --data-binary "$(json password "$pass")" | python3 -c '
import json, sys
d = json.load(sys.stdin)
if d.get("msg"):
    sys.exit("сервер отказал: %s" % d["msg"])
'
  ok "пароль для $email установлен"
  if [ "$generated" = 1 ]; then
    printf '\n   \033[1m%s\033[0m\n\n' "$pass"
    printf '   Передайте по защищённому каналу и попросите сменить в приложении.\n'
    printf '   Больше этот пароль нигде не сохранён.\n\n'
  fi
}

# Готовность определяем по тексту статуса, а не по {{.Health}}: этот плейсхолдер
# поддерживают не все версии compose и на старых он молча пустой.
stack_settling() {
  dcq ps --format '{{.Status}}' | grep -qE 'starting|unhealthy|Restarting'
}

cmd_up() {
  head_ "Обновление образов"; dc pull
  head_ "Запуск";             dc up -d
  head_ "Ожидание готовности"
  local i
  for i in $(seq 1 60); do
    if ! stack_settling; then ok "стек готов"; cmd_status; return 0; fi
    sleep 5
  done
  warn "за 5 минут не всё поднялось:"
  dcq ps --format '{{.Service}}|{{.Status}}' | awk -F'|' '{printf "   %-18s %s\n",$1,$2}'
  return 1
}

cmd_pin() {
  local v="${1:-}"
  [ -n "$v" ] || die "укажите версию: $0 pin 0.9.172"
  if grep -q '^APPFLOWY_CLOUD_VERSION=' .env; then
    sed -i "s/^APPFLOWY_CLOUD_VERSION=.*/APPFLOWY_CLOUD_VERSION=$v/" .env
  else
    printf 'APPFLOWY_CLOUD_VERSION=%s\n' "$v" >> .env
  fi
  ok "версия закреплена: $v"
  echo
  warn "откат на младшую версию НЕ откатывает миграции БД."
  printf '   Если схема уже новее, стек не поднимется, и нужен чистый старт:\n'
  printf '     cd %s && docker compose down -v && %s up\n' "$AF_DIR" "$0"
  printf '   down -v сносит тома ТОЛЬКО этого проекта. docker system prune\n'
  printf '   не запускайте: он убьёт и соседние сервисы на этой машине.\n\n'
  printf 'Применить сейчас (docker compose pull && up -d)? [y/N] '
  local a; read -r a
  if [ "$a" = y ] || [ "$a" = Y ]; then
    cmd_up
  else
    echo "не применено - запустите $0 up когда будете готовы"
  fi
}

cmd_backup() {
  mkdir -p "$BACKUP_DIR"; chmod 700 "$BACKUP_DIR"
  local stamp tmp out vol
  stamp="$(date +%Y%m%d-%H%M%S)"
  tmp="$(mktemp -d)"
  out="$BACKUP_DIR/appflowy-$stamp.tar.gz"
  trap 'rm -rf "$tmp"' EXIT

  dcq exec -T postgres pg_dump -U "$PGUSER" -d "$PGDB" > "$tmp/postgres.sql"
  [ -s "$tmp/postgres.sql" ] || die "дамп postgres пустой, бэкап не создан"

  # Вложения и обложки лежат в томе minio; имя тома зависит от имени проекта,
  # поэтому находим его, а не собираем из названия каталога.
  vol="$(docker volume ls -q | grep -E '_minio_data$' | head -1 || true)"
  if [ -n "$vol" ]; then
    docker run --rm -v "$vol:/data:ro" -v "$tmp:/out" alpine \
      tar czf /out/minio.tar.gz -C /data . >/dev/null 2>&1 || warn "не удалось сохранить файлы minio"
  else
    warn "том minio не найден - вложения в бэкап не попали"
  fi

  cp .env "$tmp/env.backup"
  [ -d nginx/ssl ] && cp -r nginx/ssl "$tmp/ssl"

  tar czf "$out" -C "$tmp" .
  chmod 600 "$out"
  ok "бэкап: $out ($(du -h "$out" | cut -f1))"

  ls -1t "$BACKUP_DIR"/appflowy-*.tar.gz 2>/dev/null | tail -n +$((BACKUP_KEEP + 1)) | xargs -r rm -f
  printf '   копий хранится: %s (лимит %s)\n' \
    "$(ls -1 "$BACKUP_DIR"/appflowy-*.tar.gz 2>/dev/null | wc -l | tr -d ' ')" "$BACKUP_KEEP"
  warn "в архиве лежит .env со всеми секретами - каталог держите закрытым"
  warn "и увозите копию с сервера: сгоревший диск уносит и данные, и бэкап"
}

cmd_install_cron() {
  local self line
  self="$(readlink -f "$0")"
  line="30 4 * * * AF_DIR=$AF_DIR $self backup >> /var/log/afctl-backup.log 2>&1"
  ( crontab -l 2>/dev/null | grep -vF "$self backup" || true; echo "$line" ) | crontab -
  ok "ежедневный бэкап в 04:30 включён"
  printf '   проверить: crontab -l\n   лог: /var/log/afctl-backup.log\n'
}

# Показывает в реальном времени только то, что относится к лимитам, лицензии и
# приглашениям. Запускается перед тем, как в приложении нажать "пригласить":
# так видно, упирается приглашение в лимит или падает по другой причине.
cmd_watch() {
  printf 'Слежу за логами. Пригласите человека в приложении.\n'
  printf 'Выход - Ctrl+C.\n\n'
  docker compose logs -f --since 10s appflowy_cloud gotrue 2>/dev/null \
    | grep --line-buffered -iE 'limit|license|plan|invit|member|guest|ERROR' \
    | grep --line-buffered -v 'variable is not set'
}

cmd_logs() {
  local svc="${1:-}" n="${2:-80}"
  if [ -n "$svc" ]; then dcq logs --tail "$n" "$svc"; else dcq logs --tail "$n"; fi
}

case "${1:-status}" in
  status)       cmd_status ;;
  users)        cmd_users ;;
  passwd)       shift; cmd_passwd "$@" ;;
  up)           cmd_up ;;
  pin)          shift; cmd_pin "$@" ;;
  watch)        cmd_watch ;;
  backup)       cmd_backup ;;
  install-cron) cmd_install_cron ;;
  logs)         shift; cmd_logs "$@" ;;
  -h|--help|help) usage ;;
  *) printf 'неизвестная команда: %s\n\n' "$1"; usage; exit 1 ;;
esac
