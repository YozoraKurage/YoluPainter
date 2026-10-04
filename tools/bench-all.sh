#!/usr/bin/env bash
# 同じ入力の既存ベンチを順番に実行し、Markdown にまとめる。
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
exec python3 tools/bench-all.py "$@"
