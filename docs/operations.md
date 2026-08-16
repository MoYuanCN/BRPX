# 安装、升级、备份与卸载

## 目录

| 路径 | 内容 |
|---|---|
| `/opt/brpx/brpx` | 服务二进制 |
| `/opt/brpx/uninstall.sh` | 卸载器 |
| `/var/lib/brpx/config.json` | 运行配置 |
| `/var/lib/brpx/data/brpx.db` | 管理、规则和审计数据 |
| `/var/lib/brpx/certificates/` | 内置 TLS 证书 |
| `/var/lib/brpx/backups/` | 安装器升级备份 |
| `/etc/systemd/system/brpx.service` | systemd 服务 |

## 升级

在新版本源码目录重新执行 `sudo ./install.sh`。安装器会先停止服务，再备份二进制、配置以及 SQLite 主文件/WAL 文件，保留现有配置，最后替换程序并执行健康检查。

查看状态与日志：

```bash
sudo systemctl status brpx
sudo journalctl -u brpx -f
curl http://127.0.0.1:2662/api/health?area=cn\&type=playurl
```

## 备份

建议停止服务后备份整个状态目录：

```bash
sudo systemctl stop brpx
sudo tar -C /var/lib -czf brpx-backup.tar.gz brpx
sudo systemctl start brpx
```

恢复时先停止服务，将目录恢复到 `/var/lib/brpx`，确认所有者为 `brpx:brpx`，再启动服务。

## 卸载

默认卸载程序和 systemd 服务，保留配置、规则、审计和备份：

```bash
sudo /opt/brpx/uninstall.sh
```

彻底删除所有 BRPX 数据：

```bash
sudo /opt/brpx/uninstall.sh --purge
```

`--purge` 需要再次输入 `PURGE`，删除后卸载器无法恢复数据。

## 常见问题

- 启动错误 78：配置文件不存在或无法解析，检查工作目录下 `config.json`。
- Redis 异常：检查 `systemctl status redis-server` 和配置中的 Redis URL。
- Web 保存提示重启：执行 `sudo systemctl restart brpx`。
- IP 规则不匹配：确认反向代理地址在 `trusted_proxies` 中，并检查转发头。
- TV 请求未进入 BRPX：确认客户端实际请求的 Host、DNS、TLS 信任和 `/pgc/player/api/playurltv` 路径。
