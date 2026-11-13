#!/usr/bin/env bash
# 给端到端测试起一个真的后端：开发数据库实例上单独的一个库（每次清空重建），临时的数据目录。
# 由 playwright.config.ts 调用，数据库地址、端口和数据目录都从环境变量来。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

: "${DATABASE_URL:?}" "${AI_INTERVIEW_BIND:?}" "${AI_INTERVIEW_DATA_DIR:?}"
db_name="${DATABASE_URL##*/}"

scripts/dev-db.sh start >/dev/null
scripts/dev-db.sh psql -q \
  -c "DROP DATABASE IF EXISTS $db_name WITH (FORCE)" \
  -c "CREATE DATABASE $db_name"

cargo build -p interview-server
cargo build -p interview-room --example demo

# 不调大模型：面试间没接语音识别时本来就不判断回答，这里把密钥清掉，面试后的分析也不会花钱
export DEEPSEEK_API_KEY=""
# 录像一秒切一片，断网补传这类情况几秒钟就能测出来
export AI_INTERVIEW_ROOM_RECORDING_TIMESLICE_MS=1000
# 语音识别是假后端（听到声音就给一句固定的话）。让它也做现场转写，
# 这样能看到浏览器录出来的分片确实解得出声音、文字确实记进了对话
export AI_INTERVIEW_STT_BACKEND=fake
export AI_INTERVIEW_ROOM_FAKE_TRANSCRIPT=true
export AI_INTERVIEW_LOG="${AI_INTERVIEW_LOG:-warn}"
exec target/debug/interview-server serve
