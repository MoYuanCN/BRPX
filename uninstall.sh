#!/usr/bin/env bash
set -Eeuo pipefail

readonly SERVICE_NAME="brpx"
readonly SERVICE_USER="brpx"
readonly INSTALL_DIR="${BRPX_INSTALL_DIR:-/opt/brpx}"
readonly STATE_DIR="${BRPX_STATE_DIR:-/var/lib/brpx}"
readonly UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"

PURGE=0
ASSUME_YES=0

usage() {
    cat <<'EOF'
Usage: uninstall.sh [--purge] [--yes]

  --purge  同时删除配置、审计数据库和备份；默认保留 /var/lib/brpx
  --yes    不显示确认提示
EOF
}

for argument in "$@"; do
    case "${argument}" in
        --purge) PURGE=1 ;;
        --yes|-y) ASSUME_YES=1 ;;
        --help|-h) usage; exit 0 ;;
        *) printf 'Unknown option: %s\n' "${argument}" >&2; usage; exit 2 ;;
    esac
done

[[ "${EUID}" -eq 0 ]] || { printf '请使用 sudo 运行卸载器\n' >&2; exit 1; }

if [[ "${ASSUME_YES}" -ne 1 ]]; then
    if [[ "${PURGE}" -eq 1 ]]; then
        printf '将删除 BRPX 程序、配置、审计和备份。输入 PURGE 继续: '
        read -r confirmation
        [[ "${confirmation}" == "PURGE" ]] || { printf '已取消\n'; exit 0; }
    else
        printf '将卸载 BRPX 程序并保留 %s。继续? [y/N] ' "${STATE_DIR}"
        read -r confirmation
        [[ "${confirmation}" =~ ^[Yy]$ ]] || { printf '已取消\n'; exit 0; }
    fi
fi

systemctl disable --now "${SERVICE_NAME}.service" 2>/dev/null || true
rm -f -- "${UNIT_PATH}"
systemctl daemon-reload
systemctl reset-failed "${SERVICE_NAME}.service" 2>/dev/null || true

rm -f -- "${INSTALL_DIR}/brpx" "${INSTALL_DIR}/update.sh" "${INSTALL_DIR}/uninstall.sh"
rmdir -- "${INSTALL_DIR}" 2>/dev/null || true

if [[ "${PURGE}" -eq 1 ]]; then
    [[ "${STATE_DIR}" == "/var/lib/brpx" || "${STATE_DIR}" == /*/brpx ]] \
        || { printf '拒绝删除非 BRPX 数据目录: %s\n' "${STATE_DIR}" >&2; exit 1; }
    rm -rf -- "${STATE_DIR}"
    userdel "${SERVICE_USER}" 2>/dev/null || true
    printf 'BRPX 已彻底卸载，数据不可由卸载器恢复。\n'
else
    printf 'BRPX 已卸载，配置、审计和备份保留在 %s。\n' "${STATE_DIR}"
    printf '重新安装时会自动复用这些数据。\n'
fi
