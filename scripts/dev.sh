#!/usr/bin/env bash
# 本地一键启动：起开发数据库，跑迁移，启动后端。
#
#   scripts/dev.sh            启动后端（http://127.0.0.1:8080，接口文档在 /api/docs）
#   scripts/dev.sh --stop-db  停掉开发数据库
#
# 第一次启动会建一个管理员账号 admin@example.com，密码打印在终端里（只在本机用）。
# 想用自己的配置：复制 .env.example 为 .env 再改。设了 DATABASE_URL 就不会起开发数据库。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

if [ "${1:-}" = "--stop-db" ]; then
  exec scripts/dev-db.sh stop
fi

# 环境变量或 .env 里已经设了的，就不用默认值
is_set() {
  [ -n "${!1:-}" ] || grep -qs "^$1=." .env
}

if ! is_set DATABASE_URL; then
  scripts/dev-db.sh start
  DATABASE_URL="$(scripts/dev-db.sh url)"
  export DATABASE_URL
fi

# 本机开发用的管理员。密码每台机器随机生成一次，存在 .runtime/ 里（不进 Git）
if ! is_set AI_INTERVIEW_ADMIN_EMAIL; then
  mkdir -p .runtime
  if [ ! -s .runtime/dev-admin-password ]; then
    (umask 077 && head -c 18 /dev/urandom | base64 | tr -d '/+=' >.runtime/dev-admin-password)
  fi
  export AI_INTERVIEW_ADMIN_EMAIL="admin@example.com"
  AI_INTERVIEW_ADMIN_PASSWORD="$(cat .runtime/dev-admin-password)"
  export AI_INTERVIEW_ADMIN_PASSWORD
  echo "本机开发账号：admin@example.com  密码：$AI_INTERVIEW_ADMIN_PASSWORD"
fi

exec cargo run -p interview-server -- serve
