#!/bin/bash
# 本地开发：构建前端 + 运行 Rust 后端
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

# 构建前端（如果 dist 不存在）
if [ ! -d "web/dist" ]; then
    echo "Building frontend..."
    cd web && npm ci && npm run build && cd ..
fi

# 运行
cargo run "$@"
