# BRPX 调试指南

> 本文用于定位“编译不了、启动不了、请求不对、管理页异常、规则/审计不生效”等常见问题。

## 1. 日志

本地提高日志级别：

```bash
RUST_LOG=debug cargo run --locked
```

systemd：

```bash
sudo journalctl -u brpx -f
sudo journalctl -u brpx -n 100 --no-pager
```

请求层日志有意只保留是否启用代理和状态码，后台日志只保留定位所需的区域、剧集与错误类型。新增日志时不要打印 access key、刷新令牌、代理端点、签名、Cookie、完整请求 URL、上游原始响应或密码；发现遗留路径违反这一约束时应按安全问题处理。

## 2. 启动问题

```text
process exits
|-- code 78 -> active config missing/invalid
|-- bind error -> port occupied or permission
|-- TLS fallback -> certificate files invalid
`-- service restarts -> inspect journal first failure
```

基本检查：

```bash
pwd
ls -l config.json data/
redis-cli ping
curl -i http://127.0.0.1:2662/
```

systemd WorkingDirectory 是 `/var/lib/brpx`，手工运行二进制时也必须从含配置的目录启动。

## 3. 管理页

| 现象 | 排查 |
|---|---|
| `/admin/` 404 | 确认运行的是新二进制，路由已注册 |
| 图标缺失 | 请求 `/admin/assets/lucide.min.js` 应为 200 |
| API 401 | 会话不存在/过期，重新登录 |
| API 403 | 写请求缺 `X-BRPX-CSRF` |
| 保存后仍是旧端口 | 监听器配置需要重启 |
| 掩码变成真实 `********` | 检查配置保存测试和实际文件；正常实现会恢复原值 |

浏览器改动后重新编译、重启并 reload。控制台应没有 error/warn。

## 4. Redis 与 SQLite

Redis：

```bash
redis-cli -u redis://127.0.0.1:6379 ping
sudo systemctl status redis-server
```

SQLite 位于 `data/brpx.db`，启用 WAL。不要在运行时只复制主文件作为备份。规则删除失败时先确认数据库是否来自旧版或被外部工具持有；当前代码会在事务内把审计外键置空再删除规则。

管理页健康状态来自实际上游请求的报告，不是独立的周期探测；`HealthCheck` 任务类型当前没有调用者。

## 5. 规则不生效

按顺序检查：

1. 规则是否启用且未过期。
2. scope 是否与请求类型一致。
3. UID 是否已在当前 Handler 阶段解析出来。
4. token 是否保存为 SHA-256 指纹。
5. IP 是否来自可信代理链。
6. 是否有更具体的 allow 规则覆盖 deny。

精确程度为 token > UID > IP > CIDR；同等程度 deny 优先。

## 6. 审计不完整

| 现象 | 说明 |
|---|---|
| 记录存在但无番剧标题 | 异步补全尚未完成或上游失败 |
| 请求数少于真实流量 | 查看“审计丢弃”计数；队列满时事件会被丢弃 |
| 管理请求没有记录 | `/admin` 被设计为排除 |
| 业务失败却 HTTP 200 | 看 `business_code`，Bilibili 常用 200 + 非零 code |
| 标题长期缺失 | 检查到 `api.bilibili.com` 的网络/DNS |

## 7. TV 路由

先确认实际请求：

```text
Host: api.snm0516.aisee.tv
Path: /pgc/player/api/playurltv
```

DNS 指向 BRPX 不代表 HTTPS 自动可用。证书必须覆盖域名并被 TV 信任；证书固定可能仍阻止代理。使用脱敏请求测试，命令历史中不要放有效 access key。

## 8. 编译与测试

| 报错 | 处理 |
|---|---|
| `link.exe not found` | Windows 安装 MSVC Build Tools 或使用 GNU target |
| 格式检查失败 | `cargo fmt --all` 后审查差异 |
| 锁文件不一致 | 有意变更依赖后更新 Cargo.lock |
| ShellCheck SC2155 | 声明和命令替换分开，保留失败状态 |
| Clippy 大量旧告警 | 新模块不得增加；不要无范围批量 `--fix` |

## 9. 最小诊断包

反馈问题时提供版本/commit、脱敏配置相关字段、请求路径（无查询秘密）、HTTP/业务码、对应日志和可复现步骤。不要上传 `data/brpx.db`、完整 config、Cookie 或 token。

检查清单：先复现，再缩小到路由/配置/存储/上游；每次只改变一个变量；修复后补自动化测试。
