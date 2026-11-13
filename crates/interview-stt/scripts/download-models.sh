#!/usr/bin/env bash
# 下载语音识别用的模型（约 170 MB），并核对 SHA-256。已经下好且校验通过的不会重下。
#
# 用法：crates/interview-stt/scripts/download-models.sh [模型目录]
# 模型目录默认取 AI_INTERVIEW_STT_MODEL_DIR，再不然是 ~/.cache/ai-interview/stt-models。
# 下载地址可以用 AI_INTERVIEW_STT_MODEL_BASE_URL 换成镜像（目录结构要和上游的发布页一样）。
#
# 模型：X-ASR 中英识别（zipformer transducer，int8，2026-06-03）和 Silero VAD，
# 都来自 sherpa-onnx 的发布页，许可分别是 Apache-2.0 和 MIT。
set -euo pipefail

dir="${1:-${AI_INTERVIEW_STT_MODEL_DIR:-$HOME/.cache/ai-interview/stt-models}}"
base="${AI_INTERVIEW_STT_MODEL_BASE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models}"
asr="sherpa-onnx-x-asr-zipformer-transducer-zh-en-punct-int8-2026-06-03"

# 文件（相对模型目录）和它的 SHA-256
checksums="\
7f6aa62056efd8af9da13e0faa81cd3f284d2fb2e3b63de56fd2dfd3450910dc  $asr/encoder-epoch-99-avg-1.int8.onnx
72f47405d3c1033bebccbef82f90071e7b4ba3e71b9c986f2b74244b25723aed  $asr/decoder-epoch-99-avg-1.onnx
aedb7fa697b2ab43f20499826fff7c997eea7d67db77be97769aeeeb726e63b3  $asr/joiner-epoch-99-avg-1.int8.onnx
b818a60878b9aae978cbb8ad594acbd403d76d1af2e31ef4197c84e2dbdba27c  $asr/tokens.txt
9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6  silero_vad.onnx"
archive_sha="5d02c36d7b44e886b7c8f0d8e051f8713acab96c264bb6ef9e718be39a6a2224"

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# 核对一组文件，全部对得上返回 0。$1 是要匹配的路径前缀
verified() {
  local sha file
  while read -r sha file; do
    case "$file" in "$1"*) ;; *) continue ;; esac
    [ -f "$dir/$file" ] || return 1
    [ "$(sha256_of "$dir/$file")" = "$sha" ] || return 1
  done <<<"$checksums"
}

mkdir -p "$dir"

if verified silero_vad.onnx; then
  echo "VAD 模型已经在了"
else
  echo "下载 VAD 模型……"
  curl -fL --retry 3 -o "$dir/silero_vad.onnx.part" "$base/silero_vad.onnx"
  mv "$dir/silero_vad.onnx.part" "$dir/silero_vad.onnx"
fi

if verified "$asr/"; then
  echo "识别模型已经在了"
else
  echo "下载识别模型（约 130 MB）……"
  archive="$dir/$asr.tar.bz2.part"
  curl -fL --retry 3 -o "$archive" "$base/$asr.tar.bz2"
  if [ "$(sha256_of "$archive")" != "$archive_sha" ]; then
    echo "下载的压缩包校验不对，已删除：$archive" >&2
    rm -f "$archive"
    exit 1
  fi
  tar -xjf "$archive" -C "$dir"
  rm -f "$archive"
fi

if verified ""; then
  echo "模型就绪：$dir"
  echo "运行时设 AI_INTERVIEW_STT_BACKEND=sherpa 和 AI_INTERVIEW_STT_MODEL_DIR=$dir"
else
  echo "模型文件校验不对，删掉 $dir 后重新下载" >&2
  exit 1
fi
