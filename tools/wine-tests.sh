#!/usr/bin/env bash
# Windows の画面なし試験。生成物と専用 Wine 環境は target/ 内だけに置く。
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
exec python3 tools/wine-tests.py "$@"
