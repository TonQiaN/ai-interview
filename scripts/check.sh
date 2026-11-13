#!/usr/bin/env bash
# 全量检查，和 CI 跑的一样。提 PR 前跑一遍。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

scripts/verify-repo-hygiene.sh

# 要数据库的测试（#[sqlx::test]）从 DATABASE_URL 找库。没设的话用本地开发库
if [ -z "${DATABASE_URL:-}" ]; then
  scripts/dev-db.sh start
  DATABASE_URL="$(scripts/dev-db.sh url)"
  export DATABASE_URL
fi

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 前端。没装 pnpm 就跳过（CI 里一定会跑）
if command -v pnpm >/dev/null 2>&1; then
  pnpm -C web install --frozen-lockfile
  pnpm -C web check
else
  echo "没找到 pnpm，跳过前端检查。安装：corepack enable pnpm"
fi
