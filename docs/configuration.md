# BRPX 配置指南

## 首次配置

安装完成后访问 `http://服务器地址:2662/admin/`。首次访问会要求创建至少 12 个字符的管理员密码。密码使用 Argon2 哈希保存于 `data/brpx.db`，不会写入 JSON/YAML 配置。

管理端读取当前运行配置并将秘密显示为 `********`。未修改的掩码字段在保存时会保留原值；要更换秘密，请直接输入新值。

## 配置文件

BRPX 按以下顺序读取第一个存在的文件：

1. `config.json`
2. `config.yml`
3. `config.yaml`

同一实例只应保留一个配置文件。安装器使用 `/var/lib/brpx/config.json`。Web 保存前会写入同目录备份，例如 `config.json.bak`，校验失败不会替换现有文件。

版本 1-4 的配置会在启动时迁移到版本 5。迁移会识别旧字段 `port`、`woker_num` 和 `resign_api_policy`。旧哔哩漫游黑名单字段会被忽略，访问规则需要在 Web 中重新建立。

## 生效方式

以下修改保存后需要重启服务：

- Redis URL
- HTTP/HTTPS 端口
- Worker 数量
- TLS 与 HTTP 跳转
- 全局请求速率和突发容量

上游地址、地区代理、缓存、重签名、通知、访问规则、可信代理和审计保留期可由运行期配置快照读取。管理端保存结果会明确返回是否需要重启。

## 关键字段

| 字段 | 含义 | 默认值 |
|---|---|---|
| `redis` | Redis 连接 URL | `redis://127.0.0.1:6379` |
| `worker_num` | Actix Worker 数 | `8` |
| `http_port` | HTTP 监听端口 | `2662` |
| `https_port` | 内置 HTTPS 监听端口 | `2663` |
| `rate_limit_per_second` | 单个识别主体每秒请求数 | `3` |
| `rate_limit_burst` | 突发容量 | `20` |
| `trusted_proxies` | 可提供转发 IP 头的 IP/CIDR | 本机回环地址 |
| `audit_retention_days` | 审计保留天数 | `30` |
| `*_tv_playurl_api` | 各地区 TV 播放上游 | `api.snm0516.aisee.tv` |

## 地区上游与代理

`cn`、`hk`、`tw`、`th` 分别配置 APP、Web、TV 播放和搜索上游。地区代理由对应的 `*_proxy_*_open` 与 `*_proxy_*_url` 配对控制。代理地址不含协议时按 SOCKS5 处理；建议显式填写 `socks5://`、`http://` 或 `https://`。

修改上游 URL 前先确认接口路径和参数语义。`D:\project\bilibili-API-collect` 仅作为开发参考，不是运行时依赖。

## 重签名和 Access Key

重签名按地区编号配置：`1=CN`、`2=HK`、`3=TW`、`4=TH`。`resign_open` 控制是否启用，`resign_pub` 控制是否向普通用户提供，`resign_from_api_open` 控制是否从自定义 API 获取。

`resign_api_sign`、通知令牌、Redis 密码和 `api_sign` 都属于秘密。不要将实际值写入示例、Issue、日志或截图。

## 可信代理

只有请求的 TCP 对端匹配 `trusted_proxies` 时，BRPX 才解析 `X-Forwarded-For` 或 `X-Real-IP`。反向代理和 BRPX 在同一主机时保留 `127.0.0.1`/`::1`；跨主机部署时填写代理的精确 IP 或最小 CIDR，不要配置 `0.0.0.0/0`。

Nginx 示例：

```nginx
location / {
    proxy_pass http://127.0.0.1:2662;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```
