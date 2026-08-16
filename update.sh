#!/usr/bin/env bash
set -Eeuo pipefail

readonly STATE_DIR="${BRPX_STATE_DIR:-/var/lib/brpx}"
readonly SOURCE_RECORD="${STATE_DIR}/install-source"
readonly DEFAULT_REPOSITORY="https://github.com/MoYuanCN/BRPX"
readonly DEFAULT_REF="main"

CHECK_ONLY=0
FORCE=0
REQUESTED_REPOSITORY="${BRPX_REPOSITORY:-}"
REQUESTED_REF="${BRPX_VERSION:-}"
BUILD_DIR=""

log() {
    printf '[BRPX Update] %s\n' "$*"
}

fail() {
    printf '[BRPX Update] ERROR: %s\n' "$*" >&2
    exit 1
}

cleanup() {
    if [[ -n "${BUILD_DIR}" && -d "${BUILD_DIR}" ]]; then
        rm -rf -- "${BUILD_DIR}"
    fi
}
trap cleanup EXIT

usage() {
    cat <<'EOF'
Usage: update.sh [--check] [--force] [--repository URL] [--ref BRANCH]

  --check           只检查是否有新版本
  --force           即使 commit 未变化也重新安装
  --repository URL  覆盖安装时记录的 Git 仓库
  --ref BRANCH      覆盖安装时记录的分支或标签
EOF
}

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --check) CHECK_ONLY=1 ;;
        --force) FORCE=1 ;;
        --repository)
            shift
            [[ "$#" -gt 0 ]] || fail "--repository 缺少参数"
            REQUESTED_REPOSITORY="$1"
            ;;
        --ref)
            shift
            [[ "$#" -gt 0 ]] || fail "--ref 缺少参数"
            REQUESTED_REF="$1"
            ;;
        --help|-h) usage; exit 0 ;;
        *) fail "未知参数: $1" ;;
    esac
    shift
done

[[ "${EUID}" -eq 0 ]] || fail "请使用 sudo 运行更新器"
[[ "$(uname -s)" == "Linux" ]] || fail "更新器仅支持 Linux"
command -v git >/dev/null 2>&1 || fail "缺少 git，请先安装"
command -v systemctl >/dev/null 2>&1 || fail "当前系统不是 systemd 环境"

record_value() {
    local key="$1"
    [[ -r "${SOURCE_RECORD}" ]] || return 0
    sed -n "s/^${key}=//p" "${SOURCE_RECORD}" | head -n 1
}

repository="${REQUESTED_REPOSITORY:-$(record_value repository)}"
repository="${repository:-${DEFAULT_REPOSITORY}}"
source_ref="${REQUESTED_REF:-$(record_value ref)}"
source_ref="${source_ref:-${DEFAULT_REF}}"
installed_commit="$(record_value commit)"
installed_commit="${installed_commit:-unknown}"
repository_display="$(printf '%s' "${repository}" | sed -E 's#(https?://)[^/@]+@#\1#')"

remote_commit="$(
    git ls-remote "${repository}" \
        "refs/heads/${source_ref}" \
        "refs/tags/${source_ref}" \
        "refs/tags/${source_ref}^{}" \
        | awk 'NR == 1 { first = $1 } /\^\{\}$/ { peeled = $1 } END { if (peeled != "") print peeled; else print first }'
)"
[[ -n "${remote_commit}" ]] || fail "找不到远端分支或标签: ${source_ref}"

log "仓库: ${repository_display}"
log "版本: ${source_ref}"
log "已安装 commit: ${installed_commit}"
log "远端 commit: ${remote_commit}"

if [[ "${installed_commit}" == "${remote_commit}" && "${FORCE}" -ne 1 ]]; then
    log "当前已经是最新版本"
    exit 0
fi

if [[ "${CHECK_ONLY}" -eq 1 ]]; then
    log "发现可用更新"
    exit 0
fi

BUILD_DIR="$(mktemp -d -t brpx-update.XXXXXX)"
log "下载更新源码"
git clone --depth 1 --branch "${source_ref}" "${repository}" "${BUILD_DIR}/source"
[[ -f "${BUILD_DIR}/source/install.sh" ]] || fail "更新源码缺少 install.sh"
[[ -f "${BUILD_DIR}/source/update.sh" ]] || fail "更新源码缺少 update.sh"

log "编译并安装更新；服务仅在替换前短暂停止"
BRPX_REPOSITORY="${repository}" \
BRPX_VERSION="${source_ref}" \
bash "${BUILD_DIR}/source/install.sh"

new_commit="$(record_value commit)"
log "更新完成: ${new_commit:-${remote_commit}}"
