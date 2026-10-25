#!/usr/bin/env bash
set -euo pipefail

# 构建网站管理镜像；Nginx 使用官方镜像，.vsp 安装包由 veydeck-suites 仓库维护。
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 从后端清单读取默认版本，命令行参数与 RELEASE_VERSION 可覆盖镜像标签。
read_version() {
    awk '
        /^\[package\]/ { in_package = 1; next }
        /^\[/ && in_package { exit }
        in_package && /^[[:space:]]*version[[:space:]]*=/ {
            value = $0
            sub(/^[[:space:]]*version[[:space:]]*=[[:space:]]*/, "", value)
            gsub(/["[:space:]\r]/, "", value)
            print value
            exit
        }
    ' "$REPO_ROOT/backend/Cargo.toml"
}

if [[ $# -gt 1 ]]; then
    echo "用法：$0 [版本]" >&2
    exit 1
fi

VERSION="${1:-${RELEASE_VERSION:-$(read_version)}}"
IMAGE_REPOSITORY="${IMAGE_REPOSITORY:-veydeck/website-manager}"
if [[ ! "$VERSION" =~ ^[a-zA-Z0-9_][a-zA-Z0-9_.-]{0,127}$ ]]; then
    echo "镜像版本必须为有效的 Docker 标签（1–128 位字母、数字、下划线、点或短横线，不能以点或短横线开头）。" >&2
    exit 1
fi
IMAGE_NAME="$IMAGE_REPOSITORY:$VERSION"
BUILD_ARGS=()
if [[ -n "${CARGO_REGISTRY_MIRROR:-}" ]]; then
    BUILD_ARGS+=(--build-arg "CARGO_REGISTRY_MIRROR=$CARGO_REGISTRY_MIRROR")
fi

# Dockerfile 完成前端及 Rust 的多阶段构建，宿主机只需要 Docker。
echo "构建镜像：$IMAGE_NAME"
docker build "${BUILD_ARGS[@]}" --file "$REPO_ROOT/Dockerfile" --tag "$IMAGE_NAME" "$REPO_ROOT"
echo "镜像构建完成：$IMAGE_NAME"
