#!/usr/bin/env bash
# 提前下载 sherpa-onnx 的预编译库，给「编译时连不上 GitHub」的环境用。
#
# 打开 sherpa 特性后，sherpa-onnx 的构建脚本会自己从 GitHub 下载预编译库（约 23 MB）。
# 一般不用管。但在出网要过代理、证书由代理重新签发的环境里（比如云端开发容器），
# 构建脚本不认系统证书，会下载失败。这时先跑这个脚本用 curl 下好，再按它打印的那行设环境变量。
#
# 用法：eval "$(crates/interview-stt/scripts/fetch-sherpa-lib.sh)"
set -euo pipefail

version="2.0.1"
cache="${SHERPA_ONNX_ARCHIVE_DIR:-$HOME/.cache/sherpa-onnx}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform="linux-x64"; sha="55cfadf65ba50d0afc1a802b7a337942819e51c94a5b5a384def1978f7c70871" ;;
  *)
    echo "这个脚本只给 Linux x86_64 准备了校验值。其他平台直接编译，让构建脚本自己下载。" >&2
    exit 1
    ;;
esac

archive="sherpa-onnx-v$version-$platform-static-lib.tar.bz2"
mkdir -p "$cache"
if [ ! -f "$cache/$archive" ]; then
  curl -fL --retry 3 -o "$cache/$archive.part" \
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$version/$archive" >&2
  mv "$cache/$archive.part" "$cache/$archive"
fi
actual="$(sha256sum "$cache/$archive" | cut -d' ' -f1)"
if [ "$actual" != "$sha" ]; then
  echo "预编译库校验不对，已删除：$cache/$archive" >&2
  rm -f "$cache/$archive"
  exit 1
fi
echo "export SHERPA_ONNX_ARCHIVE_DIR=$cache"
