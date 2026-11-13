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
