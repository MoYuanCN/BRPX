# BRPX 配置指南

## 首次配置

安装完成后访问 `http://服务器地址:2662/admin/`。首次访问会要求创建至少 12 个字符的管理员密码。密码使用 Argon2 哈希保存于 `data/brpx.db`，不会写入 JSON/YAML 配置。

“服务配置”默认使用按类别分组的可视化表单。每项配置同时显示字段名、用途、实际效果，以及“立即生效”或“重启生效”标记；顶部搜索可以按名称、字段或用途筛选。布尔值使用开关，数组和映射可直接增删，通知渠道可以切换类型。

“JSON”模式用于批量或高级编辑，与表单使用同一份草稿并双向同步。管理端将秘密显示为 `********`，未修改的掩码字段在保存时会保留原值；要更换秘密，请直接输入新值。代理认证地址、API 签名和通知令牌同样按秘密处理。

## 配置文件

BRPX 按以下顺序读取第一个存在的文件：

1. `config.json`
2. `config.yml`
3. `config.yaml`

同一实例只应保留一个配置文件。安装器使用 `/var/lib/brpx/config.json`。Web 保存前会写入同目录备份，例如 `config.json.bak`，校验失败不会替换现有文件。

版本 1-6 的配置会在启动时迁移到版本 7。迁移会识别旧字段 `port`、`woker_num` 和 `resign_api_policy`，移除不受支持的 `th_tv_playurl_api`，并补充默认域名地区映射。旧哔哩漫游黑名单字段会被忽略，访问规则需要在 Web 中重新建立。

## 生效方式

以下修改保存后需要重启服务：

- Redis URL
- HTTP/HTTPS 端口
- Worker 数量
- TLS 与 HTTP 跳转
- 全局请求速率和突发容量

上游地址、域名地区映射、地区代理、缓存、重签名、通知、访问规则、可信代理和审计保留期可由运行期配置快照读取。管理端保存结果会明确返回是否需要重启。

配置校验失败时，表单会定位对应字段并显示原因。配置版本由系统维护，在表单中不可编辑。

## 关键字段

| 字段 | 含义 | 默认值 |
|---|---|---|
| `redis` | Redis 连接 URL | `redis://127.0.0.1:6379` |
| `worker_num` | 每个 Actix `HttpServer` 的 Worker 数 | `8` |
| `http_port` | HTTP 监听端口 | `2662` |
| `https_port` | 内置 HTTPS 监听端口 | `2663` |
| `rate_limit_per_second` | 单个识别主体每秒请求数 | `3` |
| `rate_limit_burst` | 突发容量 | `20` |
| `trusted_proxies` | 可提供转发 IP 头的 IP/CIDR | 本机回环地址 |
| `host_area_map` | 访问域名到地区代码的映射 | 空映射，由部署者配置 |
| `audit_retention_days` | 审计保留天数 | `30` |
| `cn_tv_playurl_api` / `hk_tv_playurl_api` / `tw_tv_playurl_api` | 大陆、香港、台湾 TV 播放上游 | `api.snm0516.aisee.tv` |

## 地区上游与代理

`cn`、`hk`、`tw`、`th` 分别配置 APP、Web 播放和搜索上游；TV 播放仅支持 `cn`、`hk`、`tw`，泰区没有 TV 接口。地区代理由对应的 `*_proxy_*_open` 与 `*_proxy_*_url` 配对控制。代理地址不含协议时按 SOCKS5 处理；建议显式填写 `socks5://`、`http://` 或 `https://`。

修改上游 URL 前先确认接口路径和参数语义，并用脱敏请求验证；外部 API 文档不是运行时依赖。

## 访问域名地区映射

`host_area_map` 的键是不带协议、端口和路径的小写域名，值只能是 `cn`、`hk`、`tw` 或 `th`。BRPX 的运行时默认不设置任何访问域名；`config.example.*` 使用以下 RFC 保留域名演示写法：

| 示例域名 | 地区代码 | 用途 |
|---|---|---|
| `cn.example.com` | `cn` | 中国大陆访问域名 |
| `hk.example.com` | `hk` | 香港访问域名 |
| `tw.example.com` | `tw` | 台湾访问域名 |
| `th.example.com` | `th` | 泰区访问域名 |

`example.com` 及其子域名仅用于文档，部署时必须在 Web 管理端替换成自己的实际域名。播放和搜索请求先匹配 Host，再回退到 `area` 参数，因此域名映射可以覆盖冲突的查询参数。管理端保存后立即生效。把域名映射到 `th` 不会启用泰区 TV；泰区 TV 请求仍会被拒绝。

DNS 记录和 HTTPS 证书需要在域名服务商与反向代理中配置，不属于 BRPX 运行配置。Nginx/Caddy 必须把原始 Host 传给 BRPX。

## 搜索注入 JSON

`appsearch_remake` 和 `websearch_remake` 的键是访问域名，格式与 `host_area_map` 的键相同；APP 或 Web 搜索请求的 Host 命中对应域名时才会注入。值在配置文件中保持为 JSON 字符串，以兼容既有运行逻辑。管理端会把合法字符串解析为可折叠字段树：对象、数组、文本、数字、布尔值和空值均可直接编辑、增删或转换类型。

每个条目同时保留“原始 JSON”入口。原始内容输入后会即时解析，格式错误或根节点不是对象时会在当前条目显示原因，后端也会拒绝保存并定位到对应域名。可视化编辑产生的内容会在提交前重新序列化为 JSON 字符串，不改变配置文件的数据类型。

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
