#!/usr/bin/env bash
# 公开仓库的卫生检查：音频、模型权重、密钥、个人信息不进 Git。CI 和 scripts/check.sh 都会跑。
#
# 只报「哪个文件第几行、像什么」，不打印命中的内容，免得把密钥再写进终端和 CI 日志。
# 测试数据这样写不会被拦：手机号用 13800000000 到 13800009999，邮箱用 @example.com。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

MAX_BYTES=$((1024 * 1024))
problems=0

report() {
  echo "✗ $1"
  problems=$((problems + 1))
}

# 依赖锁和权重清单里全是哈希，不做内容扫描，也不限大小
is_lock_or_pin() {
  case "$1" in
    Cargo.lock | tts/locks/* | tts/artifacts/*) return 0 ;;
  esac
  return 1
}

RULES=(
  "疑似 API 密钥（sk-…）|(^|[^A-Za-z0-9_-])sk-[A-Za-z0-9]{20,}"
  "疑似 GitHub 令牌|gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,}"
  "疑似云厂商访问密钥|AKIA[0-9A-Z]{16}|AKID[0-9A-Za-z]{32}"
  "私钥|-----BEGIN ([A-Z0-9]+ )*PRIVATE KEY-----"
  "带密码的数据库地址|postgres(ql)?://[^[:space:]:/@]+:[^[:space:]@<\$]+@"
  "个人邮箱（测试数据请用 @example.com）|[A-Za-z0-9._%+-]+@(qq|163|126|139|sina|foxmail|gmail|hotmail|outlook|icloud|chinasoftinc)\\.(com|cn|net)"
  "疑似手机号（测试数据请用 1380000xxxx）|(^|[^0-9])1[3-9][0-9]{9}([^0-9]|\$)"
)

while IFS= read -r -d '' file; do
  [ -f "$file" ] || continue
  base="${file##*/}"

  case "$base" in
    .env.example) ;;
    .env | .env.*) report "$file：环境变量文件不进 Git（只提交 .env.example）" ;;
  esac
  case "$base" in
    *.wav | *.mp3 | *.m4a | *.flac | *.aiff | *.ogg | *.opus | *.mp4 | *.mov)
      report "$file：音频和视频不进 Git（真人音色属于个人信息）" ;;
    *.safetensors | *.pth | *.pt | *.npz | *.onnx | *.gguf | *.bin)
      report "$file：模型权重不进 Git" ;;
    *.pem | *.key | *.p12 | *.pfx | id_rsa* | id_ed25519*)
      report "$file：私钥或证书不进 Git" ;;
  esac

  is_lock_or_pin "$file" && continue

  size=$(wc -c <"$file")
  if [ "$size" -gt "$MAX_BYTES" ]; then
    report "$file：超过 1 MiB（$size 字节），大文件不进 Git"
    continue
  fi

  for rule in "${RULES[@]}"; do
    name="${rule%%|*}"
    pattern="${rule#*|}"
    while IFS= read -r hit; do
      [ -n "$hit" ] || continue
      # 约定的测试手机号放行
      if [ "$name" = "疑似手机号（测试数据请用 1380000xxxx）" ]; then
        line_no="${hit%%:*}"
        if ! sed -n "${line_no}p" "$file" | sed -E 's/1380000[0-9]{4}//g' | grep -Eq "$pattern"; then
          continue
        fi
      fi
      report "$file:${hit%%:*}：$name"
    done < <(grep -InE -e "$pattern" -- "$file" | cut -d: -f1 || true)
  done
done < <(git ls-files -z)

if [ "$problems" -gt 0 ]; then
  echo "仓库卫生检查没过：$problems 处。"
  exit 1
fi
echo "仓库卫生检查通过。"
