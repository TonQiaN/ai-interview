#!/usr/bin/env bash
# 在这台 Mac 上起（停）语音合成的两个 worker 进程：IndexTTS 一个，BreezeTTS2 一个。
#
#   scripts/tts-workers.sh start    起两个 worker，等模型加载完，打印后端要用的环境变量
#   scripts/tts-workers.sh stop     停掉
#   scripts/tts-workers.sh status   是否在跑
#   scripts/tts-workers.sh env      只打印后端要用的环境变量（worker 没在跑时什么都不打印）
#
# 真引擎只能在 Apple Silicon 的 Mac 上跑，要先按 tts/README.md 把上游代码（vendor/）和权重（models/）备好。
# worker 的参数和协议见 tts/worker/README.md。日志在 .runtime/tts-workers/ 下面。
#
# 可以调的环境变量：
#   AI_INTERVIEW_TTS_WORKER_PORTS   两个端口，逗号隔开，默认 7801,7802
#   AI_INTERVIEW_TTS_WORKER_FAKE=1  不加载模型，输出测试音（没有模型的机器上试脚本用，只要 python3）
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

REPO="$PWD"
RUN_DIR="$REPO/.runtime/tts-workers"
TOKEN_FILE="$RUN_DIR/token"
PORTS="${AI_INTERVIEW_TTS_WORKER_PORTS:-7801,7802}"
INDEX_PORT="${PORTS%%,*}"
BREEZE_PORT="${PORTS##*,}"
FAKE="${AI_INTERVIEW_TTS_WORKER_FAKE:-}"
# 加载模型最多等多久（秒）。头一次要把权重读进内存，慢的时候一两分钟
WAIT_SECONDS="${AI_INTERVIEW_TTS_WORKER_WAIT_SECONDS:-300}"

# 名字、端口、提供哪些引擎、上游代码在哪（worker 用那里的 Python 环境）
workers() {
  echo "index $INDEX_PORT index2,index25 vendor/mlx-indextts2"
  echo "breeze $BREEZE_PORT breeze2 vendor/breeze-tts-mlx"
}

pid_of() {
  local file="$RUN_DIR/$1.pid"
  [ -s "$file" ] || return 1
  local pid
  pid="$(cat "$file")"
  kill -0 "$pid" 2>/dev/null || return 1
  echo "$pid"
}

healthy() {
  curl -fsS --max-time 3 -H "Authorization: Bearer $(cat "$TOKEN_FILE")" \
    "http://127.0.0.1:$1/v1/health" >/dev/null 2>&1
}

print_env() {
  echo "AI_INTERVIEW_TTS_BACKEND=worker"
  echo "AI_INTERVIEW_TTS_WORKER_URLS=http://127.0.0.1:$INDEX_PORT,http://127.0.0.1:$BREEZE_PORT"
  echo "AI_INTERVIEW_TTS_WORKER_TOKEN=$(cat "$TOKEN_FILE")"
}

start_one() {
  local name="$1" port="$2" engines="$3" vendor="$4"
  if pid_of "$name" >/dev/null; then
    return 0
  fi
  local python dir args=(--engines "$engines" --port "$port")
  if [ -n "$FAKE" ]; then
    python="python3"
    dir="$REPO"
    args+=(--fake)
  else
    python="$REPO/$vendor/.venv/bin/python"
    dir="$REPO/$vendor"
    args+=(--preload)
    if [ ! -x "$python" ]; then
      echo "没找到 $vendor/.venv/bin/python。先按 tts/README.md 把上游代码和依赖备好。" >&2
      return 1
    fi
  fi
  (
    cd "$dir"
    AI_INTERVIEW_TTS_WORKER_TOKEN="$(cat "$TOKEN_FILE")" \
      nohup "$python" "$REPO/tts/worker/worker.py" "${args[@]}" \
      >>"$RUN_DIR/$name.log" 2>&1 &
    echo $! >"$RUN_DIR/$name.pid"
  )
}

wait_ready() {
  local name="$1" port="$2" waited=0
  until healthy "$port"; do
    if ! pid_of "$name" >/dev/null; then
      echo "worker $name 没起来，最后几行日志：" >&2
      tail -n 20 "$RUN_DIR/$name.log" >&2 || true
      return 1
    fi
    if [ "$waited" -ge "$WAIT_SECONDS" ]; then
      echo "等了 $WAIT_SECONDS 秒，worker $name 还没准备好。日志在 $RUN_DIR/$name.log" >&2
      return 1
    fi
    sleep 2
    waited=$((waited + 2))
    if [ $((waited % 20)) -eq 0 ]; then
      echo "还在加载 $name 的模型（$waited 秒）…" >&2
    fi
  done
}

start() {
  if [ -z "$FAKE" ] && [ "$(uname -s)-$(uname -m)" != "Darwin-arm64" ]; then
    echo "真的语音合成只能在 Apple Silicon 的 Mac 上跑。这台机器是 $(uname -s) $(uname -m)。" >&2
    exit 1
  fi
  mkdir -p "$RUN_DIR"
  if [ ! -s "$TOKEN_FILE" ]; then
    (umask 077 && head -c 24 /dev/urandom | base64 | tr -d '/+=' >"$TOKEN_FILE")
  fi
  while read -r name port engines vendor; do
    start_one "$name" "$port" "$engines" "$vendor"
  done < <(workers)
  while read -r name port _; do
    wait_ready "$name" "$port"
  done < <(workers)
  print_env
}

stop() {
  while read -r name _; do
    if pid="$(pid_of "$name")"; then
      kill "$pid" 2>/dev/null || true
      echo "停了 worker $name（进程 $pid）" >&2
    fi
    rm -f "$RUN_DIR/$name.pid"
  done < <(workers)
}

status() {
  local all=0
  while read -r name port engines _; do
    if pid_of "$name" >/dev/null && healthy "$port"; then
      echo "$name（$engines）在跑，端口 $port"
    elif pid_of "$name" >/dev/null; then
      echo "$name（$engines）进程在，还没准备好，端口 $port"
      all=1
    else
      echo "$name（$engines）没在跑"
      all=1
    fi
  done < <(workers)
  return "$all"
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  status) status ;;
  env)
    if [ -s "$TOKEN_FILE" ] && status >/dev/null 2>&1; then
      print_env
    fi
    ;;
  *)
    sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
    ;;
esac
