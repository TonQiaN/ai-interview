#!/usr/bin/env bash
# 在 Mac 上启动、停止 TTS worker。worker 是什么、参数和协议见同目录的 README.md。
#
#   tts/worker/run.sh start [index|breeze|all]   启动，等模型加载完才返回。不写是 index
#   tts/worker/run.sh stop  [index|breeze|all]   停掉。不写是 all
#   tts/worker/run.sh status                     哪些在跑、此刻忙不忙、有几条在排队
#   tts/worker/run.sh env                        打印后端要设的环境变量：eval "$(tts/worker/run.sh env)"
#
# index 是 IndexTTS 2 和 2.5（一个进程，端口 7801），breeze 是 BreezeTTS2（端口 7802）。
# 默认的引擎是 IndexTTS 2，只用它的话起 index 就够了。
#
# 可以用环境变量改的：
#   AI_INTERVIEW_TTS_HOME           vendor/ 和 models/ 所在的目录，默认是这个仓库的根。
#                                   在别的工作区（git worktree）里跑时指到放着模型的那个仓库
#   AI_INTERVIEW_TTS_INDEX_PORT     默认 7801
#   AI_INTERVIEW_TTS_BREEZE_PORT    默认 7802
#   AI_INTERVIEW_TTS_WORKER_HOST    默认 127.0.0.1。改成别的地址时必须设令牌
#   AI_INTERVIEW_TTS_WORKER_TOKEN   令牌，worker 和后端要一致
#   AI_INTERVIEW_TTS_WORKER_FAKE=1  不加载模型，用系统的 python3 起假引擎（没有 Mac 时测流程用）
#
# 日志和进程号在 $AI_INTERVIEW_TTS_HOME/.runtime/tts-worker/ 下（不进 Git）。
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
home="${AI_INTERVIEW_TTS_HOME:-$(cd "$here/../.." && pwd)}"
state="$home/.runtime/tts-worker"
host="${AI_INTERVIEW_TTS_WORKER_HOST:-127.0.0.1}"
fake="${AI_INTERVIEW_TTS_WORKER_FAKE:-}"
# 这个脚本自己查 worker 时连哪个地址：监听所有网卡时连本机
case "$host" in
  0.0.0.0 | ::) probe="127.0.0.1" ;;
  *) probe="$host" ;;
esac
# 等模型加载最多等多久（秒）。IndexTTS 实测 10 秒左右，第一次读盘会慢一些
load_timeout=300

usage() {
  sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

# 每个 worker 的设置：端口、引擎、Python 环境所在的目录
port_of() {
  case "$1" in
    index) echo "${AI_INTERVIEW_TTS_INDEX_PORT:-7801}" ;;
    breeze) echo "${AI_INTERVIEW_TTS_BREEZE_PORT:-7802}" ;;
  esac
}

engines_of() {
  case "$1" in
    index) echo "index2,index25" ;;
    breeze) echo "breeze2" ;;
  esac
}

vendor_of() {
  case "$1" in
    index) echo "$home/vendor/mlx-indextts2" ;;
    breeze) echo "$home/vendor/breeze-tts-mlx" ;;
  esac
}

names_of() {
  case "${1:-}" in
    index | breeze) echo "$1" ;;
    all) echo "index breeze" ;;
    *) return 1 ;;
  esac
}

pid_of() {
  local file="$state/$1.pid" pid
  [ -s "$file" ] || return 1
  pid="$(cat "$file")"
  kill -0 "$pid" 2>/dev/null || return 1
  echo "$pid"
}

health_of() {
  local auth=()
  if [ -n "${AI_INTERVIEW_TTS_WORKER_TOKEN:-}" ]; then
    auth=(-H "Authorization: Bearer $AI_INTERVIEW_TTS_WORKER_TOKEN")
  fi
  # 本机的 worker 不走代理
  curl -fsS --noproxy '*' --max-time 3 ${auth[@]+"${auth[@]}"} "http://$probe:$(port_of "$1")/v1/health" 2>/dev/null
}

start_one() {
  local name="$1" port log python dir pid began=$SECONDS
  port="$(port_of "$name")"
  log="$state/$name.log"
  if pid="$(pid_of "$name")"; then
    echo "$name 已经在跑（进程 ${pid}，端口 ${port}）"
    return 0
  fi
  if health_of "$name" >/dev/null; then
    echo "端口 $port 上已经有一个 worker 在跑，不是这个脚本起的。先把它停掉，或者换个端口。" >&2
    return 1
  fi

  mkdir -p "$state"
  local args=(--engines "$(engines_of "$name")" --host "$host" --port "$port" --state "$state")
  if [ -n "$fake" ]; then
    python="python3"
    dir="$here"
    args+=(--fake)
  else
    dir="$(vendor_of "$name")"
    python="$dir/.venv/bin/python"
    if [ ! -x "$python" ]; then
      echo "没找到 ${python}。" >&2
      echo "真引擎只能在 Apple Silicon 的 Mac 上跑，环境怎么装见 tts/README.md。" >&2
      echo "vendor/ 和 models/ 不在这个仓库下时，用 AI_INTERVIEW_TTS_HOME 指过去。" >&2
      return 1
    fi
    args+=(--models "$home/models" --preload)
  fi

  echo "启动 ${name}（$(engines_of "$name")，端口 ${port}）……"
  # 先进目录再放到后台：这样记下来的进程号就是 worker 自己的，不是外面那层 shell 的
  (
    cd "$dir"
    nohup "$python" "$here/worker.py" "${args[@]}" >>"$log" 2>&1 &
    echo $! >"$state/$name.pid"
  )

  # 加了 --preload：模型加载完才开始监听，所以能连上就是可以用了
  until health_of "$name" >/dev/null; do
    if ! pid_of "$name" >/dev/null; then
      echo "$name 没起来，日志最后几行（${log}）：" >&2
      tail -n 15 "$log" >&2
      rm -f "$state/$name.pid"
      return 1
    fi
    if [ $((SECONDS - began)) -ge "$load_timeout" ]; then
      echo "$name 等了 $load_timeout 秒还没准备好，日志在 ${log}。进程还留着，可以用 stop 停掉。" >&2
      return 1
    fi
    sleep 0.2
  done
  echo "$name 准备好了（等了 $((SECONDS - began)) 秒），日志在 $log"
}

stop_one() {
  local name="$1" pid began=$SECONDS
  if ! pid="$(pid_of "$name")"; then
    rm -f "$state/$name.pid"
    echo "$name 没在跑"
    return 0
  fi
  kill "$pid" 2>/dev/null || true
  while kill -0 "$pid" 2>/dev/null; do
    # 正常几百毫秒就退了；10 秒还没退就强行结束
    if [ $((SECONDS - began)) -ge 10 ]; then
      kill -9 "$pid" 2>/dev/null || true
      break
    fi
    sleep 0.2
  done
  rm -f "$state/$name.pid"
  echo "$name 停了"
}

status_one() {
  local name="$1" pid health
  if ! pid="$(pid_of "$name")"; then
    echo "${name}：没在跑"
    return 0
  fi
  if health="$(health_of "$name")"; then
    echo "${name}：在跑（进程 ${pid}，端口 $(port_of "$name")）$health"
  else
    echo "${name}：进程 $pid 在，但连不上（可能还在加载模型，或者令牌不对）"
  fi
}

print_env() {
  local urls="" name
  for name in index breeze; do
    if pid_of "$name" >/dev/null; then
      urls="${urls:+$urls,}http://$probe:$(port_of "$name")"
    fi
  done
  if [ -z "$urls" ]; then
    echo "没有 worker 在跑，先 tts/worker/run.sh start" >&2
    return 1
  fi
  echo "export AI_INTERVIEW_TTS_BACKEND=worker"
  echo "export AI_INTERVIEW_TTS_WORKER_URLS=$urls"
}

command="${1:-}"
case "$command" in
  start)
    names="$(names_of "${2:-index}")" || usage 2
    for name in $names; do start_one "$name"; done
    ;;
  stop)
    names="$(names_of "${2:-all}")" || usage 2
    for name in $names; do stop_one "$name"; done
    ;;
  status)
    for name in index breeze; do status_one "$name"; done
    ;;
  env)
    print_env
    ;;
  -h | --help | help)
    usage 0
    ;;
  *)
    usage 2
    ;;
esac
