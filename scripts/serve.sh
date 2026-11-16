#!/usr/bin/env bash
# 正式使用的一键启动：构建前端，编译带真语音识别的后端，在 Mac 上顺带起语音合成，然后启动服务。
#
#   scripts/serve.sh             构建并启动（Ctrl-C 停）
#   scripts/serve.sh --check     只检查环境和配置，说清楚还缺什么，不构建也不启动
#   scripts/serve.sh --no-build  不重新构建，直接用上次构建好的
#
# 配置写在 .env 里（从 .env.example 复制）。至少要有 DEEPSEEK_API_KEY；要让别的设备上的候选人打开面试链接，
# 还要把 AI_INTERVIEW_PUBLIC_URL 设成他们能访问的 https 地址。怎么部署见 docs/deploy.md。
#
# 和 scripts/dev.sh 的区别：dev.sh 给开发用，语音识别和语音合成都是假的；这里默认都用真的。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

mode="run"
build=true
for arg in "$@"; do
  case "$arg" in
    --check) mode="check" ;;
    --no-build) build=false ;;
    *)
      sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//' >&2
      exit 2
      ;;
  esac
done

problems=0
warnings=0
problem() {
  echo "✗ $*" >&2
  problems=$((problems + 1))
}
warn() {
  echo "! $*" >&2
  warnings=$((warnings + 1))
}
ok() {
  echo "✓ $*" >&2
}

# 环境变量里的值优先，其次是 .env 里的。都没有时是空的
setting() {
  local name="$1"
  if [ -n "${!name:-}" ]; then
    printf '%s' "${!name}"
    return
  fi
  [ -f .env ] || return 0
  local value
  value="$(sed -n "s/^$name=//p" .env | tail -n 1)"
  # 去掉两头的引号
  value="${value%\"}"
  value="${value#\"}"
  printf '%s' "$value"
}
is_set() {
  [ -n "$(setting "$1")" ]
}
has() {
  command -v "$1" >/dev/null 2>&1
}

# 和 scripts/dev-db.sh 找 PostgreSQL 的地方一样
has_postgres() {
  has pg_ctl && return 0
  local bin
  for bin in /opt/homebrew/opt/postgresql@*/bin/pg_ctl /usr/local/opt/postgresql@*/bin/pg_ctl \
    /usr/lib/postgresql/*/bin/pg_ctl /Applications/Postgres.app/Contents/Versions/latest/bin/pg_ctl; do
    [ -x "$bin" ] && return 0
  done
  return 1
}

# ── 要装的工具 ──
has cargo || problem "没找到 cargo。装 Rust：https://rustup.rs"
if has node && has pnpm; then
  :
else
  problem "没找到 node 或 pnpm（要 Node 22 以上和 pnpm 10）。macOS：brew install node pnpm"
fi
ffmpeg_bin="$(setting AI_INTERVIEW_FFMPEG)"
has "${ffmpeg_bin:-ffmpeg}" || problem "没找到 ffmpeg，面试中听不到候选人的回答。macOS：brew install ffmpeg；Debian/Ubuntu：apt install ffmpeg"
has pdftoppm || problem "没找到 pdftoppm，读不了 PDF 简历。macOS：brew install poppler；Debian/Ubuntu：apt install poppler-utils"

# ── 数据库 ──
# 没设 DATABASE_URL，或者设的就是仓库里那个库的地址（从 .env.example 复制来的），都用仓库里的库，启动时把它起起来
local_db=false
if ! is_set DATABASE_URL || [ "$(setting DATABASE_URL)" = "$(scripts/dev-db.sh url 2>/dev/null || true)" ]; then
  local_db=true
fi
if [ "$local_db" = false ]; then
  ok "数据库用配置里的 DATABASE_URL"
else
  if has_postgres; then
    ok "数据库用仓库里的 .runtime/pg。数据都在这个目录下，记得备份"
  else
    problem "没找到 PostgreSQL。macOS：brew install postgresql@16；Debian/Ubuntu：apt install postgresql"
  fi
fi

# ── 大模型 ──
if is_set DEEPSEEK_API_KEY; then
  ok "大模型的密钥有了"
else
  problem "没设 DEEPSEEK_API_KEY，出不了题也写不了报告。在 https://platform.deepseek.com 申请，写进 .env"
fi

# ── 候选人打开的地址 ──
public_url="$(setting AI_INTERVIEW_PUBLIC_URL)"
case "$public_url" in
  https://*) ok "面试链接的地址是 $public_url" ;;
  "" | http://localhost* | http://127.0.0.1*)
    warn "AI_INTERVIEW_PUBLIC_URL 是本机地址：面试链接只有在这台机器上才打得开。给候选人用要换成 https 地址，见 docs/deploy.md"
    ;;
  *)
    warn "AI_INTERVIEW_PUBLIC_URL 不是 https：浏览器在 http 页面上不给用摄像头和麦克风，候选人进不了面试。见 docs/deploy.md"
    ;;
esac

# ── 语音识别：没指定就用真的 ──
stt_backend="$(setting AI_INTERVIEW_STT_BACKEND)"
stt_backend="${stt_backend:-sherpa}"
stt_models="$(setting AI_INTERVIEW_STT_MODEL_DIR)"
stt_models="${stt_models:-$HOME/.cache/ai-interview/stt-models}"
features=()
if [ "$stt_backend" = "sherpa" ]; then
  features=(--features sherpa)
  ok "语音识别用真模型（模型放在 ${stt_models}，没有会自动下载，约 170 MB）"
else
  warn "语音识别是假的（AI_INTERVIEW_STT_BACKEND=${stt_backend}）：听不到候选人说了什么，报告没有内容可写"
fi

# ── 语音合成：配了就用配的；没配，在 Mac 上备好了上游代码就用 tts/worker/run.sh 起 worker ──
tts_backend="$(setting AI_INTERVIEW_TTS_BACKEND)"
tts_home="$(setting AI_INTERVIEW_TTS_HOME)"
tts_home="${tts_home:-$PWD}"
# 要起哪些 worker：空的是不起，index 是只起 IndexTTS，all 是连 BreezeTTS2 一起
start_workers=""
if [ -z "$tts_backend" ]; then
  if [ -n "${AI_INTERVIEW_TTS_WORKER_FAKE:-}" ]; then
    start_workers="all"
  elif [ -x "$tts_home/vendor/mlx-indextts2/.venv/bin/python" ]; then
    start_workers="index"
    [ ! -x "$tts_home/vendor/breeze-tts-mlx/.venv/bin/python" ] || start_workers="all"
  fi
fi
if [ "$tts_backend" = "worker" ]; then
  ok "语音合成用配置里的 worker（$(setting AI_INTERVIEW_TTS_WORKER_URLS)）"
elif [ -n "$start_workers" ] && [ -n "${AI_INTERVIEW_TTS_WORKER_FAKE:-}" ]; then
  warn "语音合成起的是假引擎的 worker（设了 AI_INTERVIEW_TTS_WORKER_FAKE）：面试官的声音是一段测试音"
elif [ "$start_workers" = "all" ]; then
  ok "语音合成用这台机器上的真引擎（启动时会起两个 worker，头一次加载模型要等一会儿）"
elif [ "$start_workers" = "index" ]; then
  warn "语音合成只备好了 IndexTTS，没找到 BreezeTTS2（vendor/breeze-tts-mlx）：用不了设计出来的音色，没设默认音色的面试合成语音会失败。怎么备好见 tts/README.md"
else
  warn "语音合成是假的：面试官的声音是一段测试音。真引擎只能在 Apple Silicon 的 Mac 上跑，怎么备好见 tts/README.md"
fi

# ── 管理员账号 ──
admin_note=""
if is_set AI_INTERVIEW_ADMIN_EMAIL; then
  ok "管理员账号用配置里的 $(setting AI_INTERVIEW_ADMIN_EMAIL)"
else
  # 和 scripts/dev.sh 用同一个账号、同一个密码文件：两个脚本连的是同一个库，管理员只在库里没有账号时建一次
  admin_note="管理员账号 admin@example.com，密码在 .runtime/dev-admin-password 里（第一次启动时生成）"
  ok "$admin_note"
fi

if [ "$problems" -gt 0 ]; then
  echo >&2
  echo "有 $problems 处要先解决。" >&2
  exit 1
fi
if [ "$mode" = "check" ]; then
  echo >&2
  if [ "$warnings" -gt 0 ]; then
    echo "能启动，但有 $warnings 处和正式使用还有差距（上面带 ! 的）。" >&2
  else
    echo "都准备好了。" >&2
  fi
  exit 0
fi

# ── 下面开始真的动手 ──
if [ "$local_db" = true ]; then
  scripts/dev-db.sh start >/dev/null
  DATABASE_URL="$(scripts/dev-db.sh url)"
  export DATABASE_URL
fi

if ! is_set AI_INTERVIEW_ADMIN_EMAIL; then
  mkdir -p .runtime
  if [ ! -s .runtime/dev-admin-password ]; then
    (umask 077 && head -c 18 /dev/urandom | base64 | tr -d '/+=' >.runtime/dev-admin-password)
  fi
  export AI_INTERVIEW_ADMIN_EMAIL="admin@example.com"
  AI_INTERVIEW_ADMIN_PASSWORD="$(cat .runtime/dev-admin-password)"
  export AI_INTERVIEW_ADMIN_PASSWORD
fi

if [ "$stt_backend" = "sherpa" ]; then
  crates/interview-stt/scripts/download-models.sh "$stt_models" >&2
  export AI_INTERVIEW_STT_BACKEND=sherpa
  export AI_INTERVIEW_STT_MODEL_DIR="$stt_models"
fi

if [ "$build" = true ]; then
  echo "构建前端…" >&2
  pnpm -C web install --frozen-lockfile >&2
  pnpm -C web build >&2
  echo "编译后端（第一次要几分钟）…" >&2
  cargo build --release -p interview-server "${features[@]}" >&2
fi
if [ ! -x target/release/interview-server ]; then
  problem "还没有构建过，去掉 --no-build 再跑一次"
  exit 1
fi
if [ ! -f web/dist/admin/index.html ]; then
  problem "前端还没有构建过，去掉 --no-build 再跑一次"
  exit 1
fi

# 这次启动时由这个脚本起的 worker。退出时只停这些：启动前就在跑的照样用，但不是这个脚本的，不去停它
started_workers=""
stop_workers() {
  local name
  for name in $started_workers; do
    tts/worker/run.sh stop "${name}" >&2 || true
  done
}

if [ -n "$start_workers" ]; then
  echo "起语音合成的 worker…" >&2
  # worker 的令牌和模型目录写在 .env 里的话，带给 run.sh
  for name in AI_INTERVIEW_TTS_HOME AI_INTERVIEW_TTS_WORKER_TOKEN; do
    if is_set "$name"; then
      export "$name=$(setting "$name")"
    fi
  done
  worker_names="index"
  [ "$start_workers" != "all" ] || worker_names="index breeze"
  workers_before="$(tts/worker/run.sh status)"
  for name in $worker_names; do
    # run.sh status 对没在跑的 worker 打印「名字：没在跑」
    if printf '%s\n' "$workers_before" | grep -q "^${name}：没在跑"; then
      started_workers="${started_workers} ${name}"
    else
      echo "${name} 的 worker 在启动前就在跑，直接用它。退出时不会停它" >&2
    fi
  done
  # 服务停了，这次起的 worker 也一起停，不留在后台占着十几 GB 内存。
  # 先设好再起：起到一半失败了，已经起来的那个也会停掉
  trap stop_workers EXIT
  tts/worker/run.sh start "$start_workers" >&2
  eval "$(tts/worker/run.sh env)"
fi

bind="$(setting AI_INTERVIEW_BIND)"
echo >&2
echo "HR 后台：http://${bind:-127.0.0.1:8080}/admin/" >&2
[ -z "$admin_note" ] || echo "$admin_note" >&2
echo >&2

# 服务放到后台再等它，不直接在前台跑，也不用 exec：
# 脚本收到终止信号（kill、关掉终端、进程管理工具停服务）时要把信号转给服务，等它收尾退出，再回来停 worker。
# 在前台跑的话，脚本被终止以后就直接走了，服务留在后台没人管，还占着端口
target/release/interview-server serve &
server=$!
trap 'kill -TERM "$server" 2>/dev/null || true' TERM INT HUP
while :; do
  status=0
  wait "$server" || status=$?
  # wait 被信号打断会提前回来，服务这时还在收尾：接着等，拿它真正的退出码
  kill -0 "$server" 2>/dev/null || break
done
exit "$status"
