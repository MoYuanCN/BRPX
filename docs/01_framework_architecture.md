# BRPX 架构总览

> 本文帮助第一次接触 BRPX 的开发者建立全局地图，知道请求、配置、缓存、规则和审计分别由谁负责。

## 0. 读前须知

BRPX 是一个自托管的哔哩哔哩解析服务器。客户端把播放、搜索、番剧或字幕请求发给 BRPX；BRPX 校验参数和访问规则，读取 Redis 缓存，必要时请求对应地区的上游，再把兼容响应返回客户端。

关键术语：

| 术语 | 含义 |
|---|---|
| Actix Web | Rust HTTP 框架，负责监听、路由和中间件 |
| Tokio | 异步运行时，调度网络请求、队列 Worker 和定时任务 |
| Redis | 播放结果、用户信息、健康状态等兼容缓存 |
| SQLite | 管理会话、本地规则、审计和剧集元数据 |
| 上游 | BRPX 实际访问的 Bilibili/BiliIntl/TV API |
| 运行时快照 | `Arc<BiliConfig>` 形式的只读配置视图 |

Rust 的所有权和类型系统适合约束共享状态，异步生态也适合代理型服务；代价是生命周期、`Send/Sync` 和错误类型比脚本语言更显式。

## 1. 整体分层

```text
APP / Web / TV client
          |
          v
+-----------------------------+
| Actix routes + middleware   |
| compression / audit / rate |
+-----------------------------+
          |
          v
+-----------------------------+
| handlers                    |
| parse / auth / local rules  |
+-----------------------------+
     |                 |
     v                 v
+-----------+     +-------------+
| Redis     |     | upstream    |
| cache     |     | Bilibili    |
+-----------+     +-------------+
          |
          v
+-----------------------------+
| response normalization      |
+-----------------------------+
          |
          +----> bounded audit queue ----> SQLite

Browser ----> /admin/ ----> management API ----> config / rules / audit
```

主要依赖方向从 HTTP 接入层指向业务模块和数据层。`handler.rs` 编排缓存、上游和访问控制；但现有 `types.rs` 还包含 Redis/队列操作，`cache` 与后台任务、`user_info` 与上游模块也有概念循环。新增代码应逐步收紧这些边界，不要假定仓库已经完全分层。

## 2. 目录结构

```text
BRPX/
|-- Cargo.toml / Cargo.lock       Rust 构建和锁定依赖
|-- config.example.json|yml       v5 配置范本
|-- install.sh / update.sh / uninstall.sh  Linux/systemd 生命周期
|-- src/
|   |-- main.rs                   进程入口、路由和 Worker
|   |-- lib.rs / mods.rs          库入口与模块清单
|   |-- html/admin.html           无构建步骤的管理应用
|   `-- mods/
|       |-- handler.rs            公共 API 请求编排
|       |-- upstream_res.rs       上游请求与响应修整
|       |-- cache.rs              Redis 缓存语义
|       |-- management.rs         管理认证、CSRF 和 API
|       |-- access_control.rs     UID/IP/CIDR/令牌规则
|       |-- audit.rs              审计队列、查询和剧集补全
|       |-- storage.rs            SQLite 初始化与表结构
|       |-- config.rs             配置加载、迁移、原子保存
|       |-- types.rs              配置、请求、响应和宏
|       `-- middleware/           压缩与审计中间件
|-- docs/                         运维和开发文档
`-- .github/workflows/            检查与 musl 构建
```

## 3. 核心模块

| 模块 | 提供能力 | 关键入口 |
|---|---|---|
| `main.rs` | 初始化 Redis/SQLite/队列，注册路由 | `main()` |
| `management.rs` | 登录、密码轮换、配置和查询 API | `configure_admin()` |
| `handler.rs` | 参数检查、用户识别、规则判定 | `handle_playurl_request()` |
| `access_control.rs` | 规则 CRUD 和确定性优先级 | `AccessControlService::evaluate()` |
| `audit.rs` | 有界写入、筛选、保留期、元数据 | `AuditService`, `run_worker()` |
| `upstream_res.rs` | 选择上游并规范返回 | `get_upstream_bili_*` |
| `types.rs` | `BiliConfig`、`BiliRuntime`、`ReqType` | 多个共享类型 |

`src/mods/stats_info.rs` 目前未由 `mods.rs` 导出且只有注释，属于未编译遗留文件，不是运行时模块。

共享状态关系：

```text
AppState
|-- Arc<RwLock<Arc<BiliConfig>>>
|-- Redis Pool
|-- Arc<Sender<BackgroundTaskType>>
|-- Database(Arc<Mutex<Connection>>)
|-- AccessControlService
`-- AuditService
```

## 4. 构建体系

项目只使用 Cargo，不存在单独的前端打包。`admin.html` 和 Lucide 脚本通过 `include_str!` 编入二进制。`Cargo.lock` 被跟踪，CI 先运行格式、检查、测试和 Clippy，再用 musl 容器生成 Linux 静态构建产物。

## 5. 示例与参考

仓库没有独立 demo。`config.example.*` 是可执行配置范本，模块内 `#[cfg(test)]` 测试是最可靠的行为示例。外部测试脚本不参与构建，也不属于项目交付物。

## 6. 快速定位

| 想了解 | 从这里开始 |
|---|---|
| 服务为何启动失败 | `src/main.rs`, `src/mods/config.rs` |
| 某播放请求走哪个上游 | `handler.rs` -> `ReqType::get_api()` |
| Web 保存如何保密 | `management.rs::redact_secrets()` |
| IP/UID 规则为何命中 | `access_control.rs::evaluate()` |
| 审计为何没有标题 | `audit.rs::fetch_episode_metadata()` |
| TV 请求兼容 | `main.rs::tvplayurl`, `docs/tv.md` |
| 安装目录和备份 | `install.sh`, `docs/operations.md` |

检查清单：先看入口和模块表，再沿一条真实请求阅读；不要从 `types.rs` 的数千行定义开始盲目向下翻。
