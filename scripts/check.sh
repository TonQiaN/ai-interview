#!/usr/bin/env bash
# 全量检查，和 CI 跑的一样。提 PR 前跑一遍。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
scripts/verify-repo-hygiene.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
