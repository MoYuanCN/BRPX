# 构建与质量门禁

> 本文说明如何从干净环境构建 BRPX、理解 Cargo/CI 配置并定位常见失败。

## 0. 构建体系是什么

Cargo 同时负责依赖解析、编译、测试和发布 Profile。BRPX 没有 Node 前端构建；管理资源在 Rust 编译时嵌入。

## 1. 必要环境

Linux 开发机通常需要：

```bash
sudo apt-get install build-essential cmake pkg-config git redis-server
rustup toolchain install stable
```

RHEL/Fedora 使用 `gcc gcc-c++ make cmake pkg-config git redis`。SQLite 使用 `rusqlite` 的 `bundled` 特性，不要求系统 SQLite 开发包。

## 2. Cargo.toml 关键项

| 项 | 用途 |
|---|---|
| `edition = "2021"` | Rust 语法与名称解析版本 |
| `actix-web` | HTTP、压缩、Rustls TLS |
| `tokio = { features=["full"] }` | 异步运行时能力 |
| `deadpool-redis` | Redis 连接池 |
| `rusqlite/bundled` | 内置 SQLite |
| `argon2` | 管理密码哈希 |
| `[profile.fast]` | 继承 release，开启 LTO 的分发构建 |

`Cargo.lock` 必须跟随应用提交，保证 CI 和发布重现同一依赖集合。

## 3. 本地命令

首次运行：

```bash
cp config.example.json config.json
cargo run --locked
```

质量门禁：

```bash
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets
shellcheck install.sh update.sh uninstall.sh
bash -n install.sh update.sh uninstall.sh
```

当前旧代码仍有较多 Clippy 风格告警，因此 CI 不使用 `-D warnings`；新增模块不应增加告警。

## 4. Debug、Release 与 Fast

| 命令 | 用途 | 特点 |
|---|---|---|
| `cargo run` | 本地开发 | 编译快、含调试信息 |
| `cargo build --release` | 常规生产 | 标准 release 优化 |
| `cargo build --profile fast` | CI 分发 | LTO、单 codegen unit，构建更慢 |

不要用 `fast` 作为日常增量构建，它为产物体积/运行性能牺牲了编译速度。

## 5. CI 调用关系

```text
push / manual
   |
   v
quality gates
   |
   v
musl Docker build
   |
   v
artifact upload
```

PR Workflow 运行格式、检查、测试、Clippy 和 ShellCheck。主 CI 在这些门禁通过后构建 `x86_64-unknown-linux-musl` 产物。

## 6. 管理资源构建

`management.rs` 在 `admin_page()` 与 `lucide_script()` 的响应体中分别调用 `include_str!("../html/admin.html")` 和 `include_str!("../html/lucide.min.js")`。

修改 HTML/JS 后必须重新编译并重启，浏览器再 reload。不存在热更新或 `npm build`。

## 7. 常见错误

| 现象 | 原因 | 修复 |
|---|---|---|
| `link.exe not found` | Windows 选择 MSVC 但无 Build Tools | 安装 MSVC，或使用已配置的 GNU target |
| `config.json` 不存在 | 运行目录无活动配置 | 复制 example 或在 systemd 状态目录运行 |
| `--locked` 失败 | Cargo.toml 与锁文件不一致 | 有意更新依赖后生成并提交 Cargo.lock |
| Shell 脚本出现 `^M` | CRLF 检出 | 保留 `.gitattributes`，重新检出 LF |
| Redis 连接失败 | 服务未启动或 URL 错 | 启动 Redis，检查 `redis` 字段 |
| musl 构建失败 | 容器/网络/本地 target 不完整 | 先确认普通 `cargo test`，再复现 CI 容器 |

## 8. 新增依赖检查清单

- 先确认标准库或现有依赖不能解决。
- 评估 MSRV、许可证、musl 和跨平台支持。
- 使用精确的主版本约束并更新 `Cargo.lock`。
- 运行全部 target 的 Clippy 和测试。
- 不把运行时秘密或下载工具加入仓库。
