# BRPX

BRPX 是一个基于 Rust、Actix Web 和 Redis 的自托管哔哩哔哩解析服务器。它提供 APP、Web、泰区和 TV 播放接口，并内置受认证保护的管理端、配置管理、本地访问规则和脱敏请求审计。

> BRPX 仅处理解析 API，不代理实际视频流。哔哩哔哩接口可能随时变化，部署者需要自行评估账号、网络和合规风险。

## 功能

- 国内、香港、台湾、泰国地区播放与搜索上游
- APP、Web、泰区字幕和 `access_key` 接口
- 大陆、香港、台湾 TV 路由 `/pgc/player/api/playurltv`（泰区不支持 TV）
- Redis 缓存、地区缓存、限流、上游代理和重签名
- Web 管理端与版本化配置
- UID、IP、CIDR、访问令牌指纹允许/拒绝规则
- 请求审计、筛选、保留期清理和番剧标题异步补全
- Linux/systemd 一键安装、快速更新、升级备份和安全卸载

## 一键安装

支持带 systemd 的 Debian/Ubuntu 与 RHEL 系发行版。安装器不会执行整机升级，也不会在命令行询问业务配置。

```bash
curl -fsSL https://raw.githubusercontent.com/MoYuanCN/BRPX/main/install.sh -o /tmp/brpx-install.sh
sudo bash /tmp/brpx-install.sh
```

安装器会：

1. 安装缺失的编译工具和 Redis。
2. 创建不可登录的 `brpx` 服务用户。
3. 安装到 `/opt/brpx`，状态保存到 `/var/lib/brpx`。
4. 创建并启动 `brpx.service`。
5. 访问本机 HTTP 端口完成健康检查。
6. 安装 `/opt/brpx/update.sh`，记录仓库、分支和 commit。

完成后访问：

```text
http://服务器地址:2662/admin/
```

首次访问创建管理员密码，随后所有业务设置均在 Web 中完成。

## 从源码运行

依赖 Rust stable 和 Redis：

```bash
git clone https://github.com/MoYuanCN/BRPX.git
cd BRPX
cp config.example.json config.json
cargo build --locked --release
./target/release/biliroaming_rust_server
```

开发模式：

```bash
cargo run
cargo test --locked
```

默认监听 `0.0.0.0:2662`，Redis 为 `redis://127.0.0.1:6379`。运行目录需要能写入配置备份和 `data/brpx.db`。

## 管理端

管理端包含四个工作区：

- 运行概览：服务版本、Redis、请求量、错误、阻断和审计队列。
- 服务配置：按类别可视化编辑全部配置，每项显示用途、效果和生效方式；高级模式可编辑 JSON。
- 访问规则：管理 UID、IP、CIDR 和访问令牌指纹规则。
- 请求审计：按 IP、UID、接口、地区和阻断结果筛选请求。

管理员密码使用 Argon2 哈希。会话 Cookie 为 `HttpOnly` 和 `SameSite=Strict`，写操作还需要 CSRF 令牌。配置秘密以 `********` 显示，未修改时保留原值。

公网部署前请阅读 [安全与访问控制](docs/security.md)。

## API

| 路径 | 用途 |
|---|---|
| `/pgc/player/api/playurl` | APP 番剧播放 |
| `/pgc/player/web/playurl` | Web 番剧播放 |
| `/pgc/player/api/playurltv` | TV 番剧播放 |
| `/intl/gateway/v2/ogv/playurl` | 国际版播放 |
| `/x/v2/search/type` | APP 搜索 |
| `/x/web-interface/search/type` | Web 搜索 |
| `/intl/gateway/v2/app/search/type` | 国际版搜索 |
| `/intl/gateway/v2/ogv/view/app/season` | 国际版番剧信息 |
| `/intl/gateway/v2/app/subtitle` | 国际版字幕 |
| `/api/accesskey` | 受签名保护的 access key API |
| `/api/health` | 地区上游健康状态 |
| `/admin/` | Web 管理端 |

API 参数应以公开接口资料和脱敏实测请求为依据；BRPX 不依赖外部 API 文档仓库或测试脚本运行。

## TV 端

TV 上游为：

```text
https://api.snm0516.aisee.tv/pgc/player/api/playurltv
```

BRPX 对 TV 路由使用显式客户端语义和独立缓存分区，不再依赖 `fnval` 猜测。客户端流量指向、DNS 和 TLS 限制见 [TV 端接入](docs/tv.md)。

## 配置

JSON 是安装器使用的规范格式，YAML 也受支持：

```bash
cp config.example.json config.json
# 或
cp config.example.yml config.yml
```

配置版本为 5。旧配置启动时会迁移并保留正确的 `worker_num`；`port`、`woker_num` 和 `resign_api_policy` 会转换为当前字段。在线哔哩漫游黑名单已完全移除，旧黑名单字段不会继续生效。

详细字段、热更新、代理、重签名、可信代理与秘密管理见 [配置指南](docs/configuration.md)。

## 访问规则

规则按主体具体程度匹配：

```text
访问令牌指纹 > UID > 精确 IP > CIDR
```

同等具体程度下拒绝优先。规则可限定到播放、TV、搜索、番剧、字幕或 access key 接口，并支持原因、停用和过期时间。原始访问令牌只在创建规则时使用，数据库只保存 SHA-256 指纹。

BRPX 只在 TCP 对端属于 `trusted_proxies` 时接受转发 IP 头，防止客户端伪造 `X-Real-IP` 绕过规则。

## 请求审计

审计记录请求 ID、时间、来源、UID、接口、地区、番剧/剧集、HTTP/业务响应码、耗时、缓存/上游和命中规则。以下内容不会保存：

- 原始 access key
- Cookie、Authorization
- 请求签名
- 完整敏感查询串
- 管理密码和配置秘密

番剧标题通过后台任务补全，上游失败不会阻塞主请求。默认保留 30 天，可在 Web 配置。

## 反向代理与 HTTPS

建议由 Nginx、Caddy 或现有网关终止 HTTPS，再代理到 `127.0.0.1:2662`。必须传递真实客户端地址，并把代理地址加入 `trusted_proxies`。

内置 HTTPS 从运行目录的以下文件加载证书：

```text
certificates/fullchain.pem
certificates/privkey.pem
```

修改监听端口、Redis、TLS、Worker 或全局限流后需要重启：

```bash
sudo systemctl restart brpx
```

## 更新与卸载

检查并安装远端更新：

```bash
sudo /opt/brpx/update.sh --check
sudo /opt/brpx/update.sh
```

更新器读取安装时记录的仓库和分支；发现新 commit 后下载源码，并复用安装器完成编译、停服备份、替换和健康检查。也可在新版源码目录再次执行 `sudo ./install.sh`。

默认卸载并保留数据：

```bash
sudo /opt/brpx/uninstall.sh
```

彻底删除数据：

```bash
sudo /opt/brpx/uninstall.sh --purge
```

完整目录、备份、恢复与故障排查见 [运维指南](docs/operations.md)。

## 开发与质量门禁

```bash
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets
shellcheck install.sh update.sh uninstall.sh
```

仓库开发规则见 [AGENTS.md](AGENTS.md)。`Cargo.lock` 必须提交，以保证应用构建可复现。

## 文档

- [01 架构总览](docs/01_framework_architecture.md)
- [02 设计思想](docs/02_framework_philosophy.md)
- [03 Rust 特性](docs/03_lang_concepts.md)
- [04 代码导读](docs/04_code_walkthrough.md)
- [05 运行时模型](docs/05_runtime_model.md)
- [06 构建指南](docs/06_build_guide.md)
- [07 新功能接入](docs/07_integration_guide.md)
- [08 调试指南](docs/08_debug_guide.md)
- [09 设计规范](docs/09_design_conventions.md)
- [配置指南](docs/configuration.md)
- [TV 端接入](docs/tv.md)
- [安全与访问控制](docs/security.md)
- [安装、升级、备份与卸载](docs/operations.md)
- [开发说明](src/docs/dev.md)
- [Redis 数据说明](src/docs/redis_data.md)

## License

本项目遵循 [GNU Affero General Public License v3.0](LICENSE)。
