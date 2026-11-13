#!/usr/bin/env bash
# 本地开发用的 PostgreSQL：在仓库的 .runtime/pg 下起一个只属于这个项目的数据库实例。
# 只监听 127.0.0.1，本机连接不需要密码。不进 Git，删掉 .runtime/pg 就回到全新状态。
#
#   scripts/dev-db.sh start    起库（没有就先初始化），打印 DATABASE_URL
#   scripts/dev-db.sh stop     停库
#   scripts/dev-db.sh status   是否在跑
#   scripts/dev-db.sh reset    停库并删掉全部数据
#   scripts/dev-db.sh url      只打印 DATABASE_URL
#   scripts/dev-db.sh psql     连进去
#
# 需要本机装了 PostgreSQL 16 或更新的版本：
#   Mac：brew install postgresql@16        Ubuntu：apt install postgresql
# 已经有 DATABASE_URL 的话不需要这个脚本，直接用自己的库。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

PORT="${AI_INTERVIEW_DEV_DB_PORT:-54329}"
DB_NAME="ai_interview"
DB_USER="ai_interview"
DATA_DIR="$PWD/.runtime/pg"
LOG_FILE="$PWD/.runtime/pg.log"
URL="postgres://$DB_USER@127.0.0.1:$PORT/$DB_NAME"

find_bin_dir() {
  if command -v pg_ctl >/dev/null 2>&1 && command -v initdb >/dev/null 2>&1; then
    dirname "$(command -v pg_ctl)"
    return
  fi
  local dir
  for dir in /opt/homebrew/opt/postgresql@*/bin /usr/local/opt/postgresql@*/bin \
    /usr/lib/postgresql/*/bin /Applications/Postgres.app/Contents/Versions/latest/bin; do
    if [ -x "$dir/pg_ctl" ]; then
      echo "$dir"
    fi
  done | sort -V | tail -n 1
}

BIN_DIR="$(find_bin_dir)"
if [ -z "$BIN_DIR" ]; then
  echo "找不到 PostgreSQL（pg_ctl、initdb）。Mac 上：brew install postgresql@16" >&2
  exit 1
fi

# PostgreSQL 不肯用 root 跑。云端容器里是 root，就换成系统的 postgres 用户。
if [ "$(id -u)" = "0" ]; then
  if ! id postgres >/dev/null 2>&1; then
    echo "当前是 root，而且没有 postgres 用户，起不了库。" >&2
    exit 1
  fi
  as_pg() { runuser -u postgres -- "$@"; }
  prepare_dir() {
    mkdir -p "$DATA_DIR"
    touch "$LOG_FILE"
    chown -R postgres "$DATA_DIR" "$LOG_FILE"
    # postgres 用户要能走到这个目录
    local dir="$PWD/.runtime"
    while [ "$dir" != "/" ]; do
      chmod o+x "$dir"
      dir="$(dirname "$dir")"
    done
  }
else
  as_pg() { "$@"; }
  prepare_dir() { mkdir -p "$DATA_DIR"; }
fi

is_running() { as_pg "$BIN_DIR/pg_ctl" -D "$DATA_DIR" status >/dev/null 2>&1; }

start() {
  prepare_dir
  if [ ! -f "$DATA_DIR/PG_VERSION" ]; then
    as_pg "$BIN_DIR/initdb" -D "$DATA_DIR" -U "$DB_USER" --auth=trust --encoding=UTF8 --locale=C >/dev/null
  fi
  if ! is_running; then
    as_pg "$BIN_DIR/pg_ctl" -D "$DATA_DIR" -l "$LOG_FILE" -w \
      -o "-p $PORT -c listen_addresses=127.0.0.1 -c unix_socket_directories=''" start >/dev/null
  fi
  if ! "$BIN_DIR/psql" -h 127.0.0.1 -p "$PORT" -U "$DB_USER" -d postgres -Atc \
    "SELECT 1 FROM pg_database WHERE datname = '$DB_NAME'" | grep -q 1; then
    "$BIN_DIR/psql" -h 127.0.0.1 -p "$PORT" -U "$DB_USER" -d postgres -qc "CREATE DATABASE $DB_NAME"
  fi
}

case "${1:-start}" in
  start)
    start
    echo "$URL"
    ;;
  stop)
    if is_running; then as_pg "$BIN_DIR/pg_ctl" -D "$DATA_DIR" -m fast -w stop >/dev/null; fi
    ;;
  status)
    if is_running; then echo "在跑：$URL"; else echo "没在跑"; exit 1; fi
    ;;
  reset)
    if is_running; then as_pg "$BIN_DIR/pg_ctl" -D "$DATA_DIR" -m fast -w stop >/dev/null; fi
    rm -rf "$DATA_DIR" "$LOG_FILE"
    ;;
  url)
    echo "$URL"
    ;;
  psql)
    shift
    exec "$BIN_DIR/psql" -h 127.0.0.1 -p "$PORT" -U "$DB_USER" -d "$DB_NAME" "$@"
    ;;
  *)
    echo "用法：scripts/dev-db.sh start|stop|status|reset|url|psql" >&2
    exit 2
    ;;
esac
