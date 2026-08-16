#!/usr/bin/env bash
set -Eeuo pipefail

readonly SERVICE_NAME="brpx"
readonly SERVICE_USER="brpx"
readonly INSTALL_DIR="${BRPX_INSTALL_DIR:-/opt/brpx}"
readonly STATE_DIR="${BRPX_STATE_DIR:-/var/lib/brpx}"
readonly UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"
readonly REPOSITORY="${BRPX_REPOSITORY:-https://github.com/MoYuanCN/BRPX}"
readonly SOURCE_REF="${BRPX_VERSION:-main}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR

BUILD_DIR=""

log() {
    printf '[BRPX] %s\n' "$*"
}

fail() {
    printf '[BRPX] ERROR: %s\n' "$*" >&2
    exit 1
}

cleanup() {
    if [[ -n "${BUILD_DIR}" && -d "${BUILD_DIR}" ]]; then
        rm -rf -- "${BUILD_DIR}"
    fi
}
trap cleanup EXIT

require_root() {
    [[ "${EUID}" -eq 0 ]] || fail "请使用 sudo 运行安装器"
    [[ "$(uname -s)" == "Linux" ]] || fail "一键安装器仅支持 Linux"
    command -v systemctl >/dev/null 2>&1 || fail "当前系统不是 systemd 环境"
}

load_os_release() {
    [[ -r /etc/os-release ]] || fail "无法识别 Linux 发行版"
    # shellcheck disable=SC1091
    source /etc/os-release
}

install_packages() {
    local packages=(curl ca-certificates git cmake pkg-config)
    if command -v apt-get >/dev/null 2>&1; then
        export DEBIAN_FRONTEND=noninteractive
        apt-get update
        apt-get install -y --no-install-recommends build-essential redis-server "${packages[@]}"
    elif command -v dnf >/dev/null 2>&1; then
        dnf install -y gcc gcc-c++ make redis "${packages[@]}"
    elif command -v yum >/dev/null 2>&1; then
        yum install -y gcc gcc-c++ make redis "${packages[@]}"
    else
        fail "仅支持 apt、dnf 或 yum；请按 README 手动安装"
    fi
}

ensure_dependencies() {
    local missing=0
    for command_name in curl git cmake redis-server cc; do
        if ! command -v "${command_name}" >/dev/null 2>&1; then
            missing=1
        fi
    done
    if [[ "${missing}" -eq 1 ]]; then
        log "安装缺失的构建与运行依赖"
        install_packages
    fi
    systemctl enable --now redis-server.service 2>/dev/null \
        || systemctl enable --now redis.service 2>/dev/null \
        || fail "Redis 服务启动失败"
}

ensure_rust() {
    if command -v cargo >/dev/null 2>&1; then
        return
    fi
    log "安装隔离的 Rust 构建工具链" >&2
    export CARGO_HOME="${STATE_DIR}/.cargo"
    export RUSTUP_HOME="${STATE_DIR}/.rustup"
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --no-modify-path >&2
    export PATH="${CARGO_HOME}/bin:${PATH}"
}

prepare_source() {
    if [[ -f "${SCRIPT_DIR}/Cargo.toml" \
        && -f "${SCRIPT_DIR}/config.example.json" \
        && -f "${SCRIPT_DIR}/uninstall.sh" \
        && -f "${SCRIPT_DIR}/update.sh" ]]; then
        printf '%s\n' "${SCRIPT_DIR}"
        return
    fi
    BUILD_DIR="$(mktemp -d -t brpx-build.XXXXXX)"
    log "下载 BRPX ${SOURCE_REF} 源码" >&2
    git clone --depth 1 --branch "${SOURCE_REF}" "${REPOSITORY}" "${BUILD_DIR}/source" >&2
    printf '%s\n' "${BUILD_DIR}/source"
}

build_binary() {
    local source_dir="$1"
    if [[ -n "${BRPX_BINARY_PATH:-}" ]]; then
        [[ -x "${BRPX_BINARY_PATH}" ]] || fail "BRPX_BINARY_PATH 不可执行"
        printf '%s\n' "${BRPX_BINARY_PATH}"
        return
    fi
    ensure_rust
    log "编译 BRPX release 版本" >&2
    cargo build --locked --release --manifest-path "${source_dir}/Cargo.toml" >&2
    printf '%s\n' "${source_dir}/target/release/biliroaming_rust_server"
}

backup_existing_installation() {
    if [[ ! -e "${INSTALL_DIR}/brpx" && ! -e "${STATE_DIR}/config.json" ]]; then
        return
    fi
    local backup_dir
    backup_dir="${STATE_DIR}/backups/$(date +%Y%m%d-%H%M%S)"
    install -d -m 0750 "${backup_dir}"
    [[ -f "${INSTALL_DIR}/brpx" ]] && cp -a "${INSTALL_DIR}/brpx" "${backup_dir}/brpx"
    [[ -f "${STATE_DIR}/config.json" ]] && cp -a "${STATE_DIR}/config.json" "${backup_dir}/config.json"
    local database_file
    for database_file in brpx.db brpx.db-wal brpx.db-shm; do
        [[ -f "${STATE_DIR}/data/${database_file}" ]] \
            && cp -a "${STATE_DIR}/data/${database_file}" "${backup_dir}/${database_file}"
    done
    log "原版本已备份到 ${backup_dir}"
}

install_files() {
    local source_dir="$1"
    local binary_path="$2"
    id -u "${SERVICE_USER}" >/dev/null 2>&1 \
        || useradd --system --home-dir "${STATE_DIR}" --shell /usr/sbin/nologin "${SERVICE_USER}"
    install -d -m 0755 "${INSTALL_DIR}"
    install -d -o "${SERVICE_USER}" -g "${SERVICE_USER}" -m 0750 \
        "${STATE_DIR}" "${STATE_DIR}/data" "${STATE_DIR}/backups" "${STATE_DIR}/certificates"
    install -o root -g root -m 0755 "${binary_path}" "${INSTALL_DIR}/brpx"
    install -o root -g root -m 0755 "${source_dir}/uninstall.sh" "${INSTALL_DIR}/uninstall.sh"
    install -o root -g root -m 0755 "${source_dir}/update.sh" "${INSTALL_DIR}/update.sh"
    if [[ ! -f "${STATE_DIR}/config.json" ]]; then
        install -o "${SERVICE_USER}" -g "${SERVICE_USER}" -m 0640 \
            "${source_dir}/config.example.json" "${STATE_DIR}/config.json"
    fi
    cat >"${STATE_DIR}/install-manifest" <<EOF
${INSTALL_DIR}/brpx
${INSTALL_DIR}/uninstall.sh
${INSTALL_DIR}/update.sh
${UNIT_PATH}
EOF
    chown "${SERVICE_USER}:${SERVICE_USER}" "${STATE_DIR}/install-manifest"
    chmod 0640 "${STATE_DIR}/install-manifest"
    local source_commit recorded_repository
    source_commit="$(git -C "${source_dir}" rev-parse HEAD 2>/dev/null || printf 'unknown')"
    recorded_repository="$(printf '%s' "${REPOSITORY}" | sed -E 's#(https?://)[^/@]+@#\1#')"
    cat >"${STATE_DIR}/install-source" <<EOF
repository=${recorded_repository}
ref=${SOURCE_REF}
commit=${source_commit}
EOF
    chown "${SERVICE_USER}:${SERVICE_USER}" "${STATE_DIR}/install-source"
    chmod 0640 "${STATE_DIR}/install-source"
}

install_service() {
    cat >"${UNIT_PATH}" <<EOF
[Unit]
Description=BRPX Bilibili Parse Server
After=network-online.target redis-server.service redis.service
Wants=network-online.target

[Service]
Type=simple
User=${SERVICE_USER}
Group=${SERVICE_USER}
WorkingDirectory=${STATE_DIR}
ExecStart=${INSTALL_DIR}/brpx
Restart=on-failure
RestartSec=3s
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ReadWritePaths=${STATE_DIR}
UMask=0027

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable --now "${SERVICE_NAME}.service"
}

verify_installation() {
    sleep 2
    if ! systemctl is-active --quiet "${SERVICE_NAME}.service"; then
        journalctl -u "${SERVICE_NAME}.service" -n 40 --no-pager >&2 || true
        fail "服务启动失败，原配置与备份仍保留在 ${STATE_DIR}"
    fi
    local port https_port
    port="$(sed -nE 's/^[[:space:]]*"http_port"[[:space:]]*:[[:space:]]*([0-9]+).*/\1/p' "${STATE_DIR}/config.json" | head -n1)"
    port="${port:-2662}"
    https_port="$(sed -nE 's/^[[:space:]]*"https_port"[[:space:]]*:[[:space:]]*([0-9]+).*/\1/p' "${STATE_DIR}/config.json" | head -n1)"
    https_port="${https_port:-2663}"
    if grep -Eq '^[[:space:]]*"https_support"[[:space:]]*:[[:space:]]*true' "${STATE_DIR}/config.json"; then
        curl -kfsS --max-time 5 "https://127.0.0.1:${https_port}/" >/dev/null \
            || curl -fsS --max-time 5 "http://127.0.0.1:${port}/" >/dev/null \
            || fail "systemd 已启动，但 HTTP/HTTPS 健康检查失败"
    else
        curl -fsS --max-time 5 "http://127.0.0.1:${port}/" >/dev/null \
            || fail "systemd 已启动，但 HTTP 健康检查失败"
    fi
    log "安装完成"
    log "管理地址: http://服务器IP:${port}/admin/"
    log "更新命令: sudo ${INSTALL_DIR}/update.sh"
    log "卸载命令: sudo ${INSTALL_DIR}/uninstall.sh"
}

main() {
    require_root
    load_os_release
    ensure_dependencies
    local source_dir binary_path
    source_dir="$(prepare_source)"
    binary_path="$(build_binary "${source_dir}")"
    [[ -x "${binary_path}" ]] || fail "编译产物不存在: ${binary_path}"
    systemctl stop "${SERVICE_NAME}.service" 2>/dev/null || true
    backup_existing_installation
    install_files "${source_dir}" "${binary_path}"
    install_service
    verify_installation
}

main "$@"
