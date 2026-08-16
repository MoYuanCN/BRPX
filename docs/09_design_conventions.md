# BRPX 设计与编码规范

> 本文把仓库现有模式整理为可执行的设计决策，帮助新增代码保持边界、安全和可维护性。

## 1. 模块选择决策树

```text
new behavior
|-- HTTP parsing/orchestration? -> handler.rs
|-- upstream protocol?         -> upstream_res.rs
|-- Redis key/TTL?             -> cache.rs / types.rs
|-- persistent control data?   -> storage.rs + dedicated service
|-- admin operation?           -> management.rs + admin.html
`-- cross-cut request concern? -> middleware/
```

一个函数如果不能用一句话描述职责，应先拆分；但不要为了减少两行重复创建没有业务含义的抽象。

## 2. 命名

| 元素 | 规则 | 示例 |
|---|---|---|
| 类型 | `UpperCamelCase` | `AuditService` |
| 函数/字段 | `snake_case` | `config_snapshot` |
| 常量 | `UPPER_SNAKE_CASE` | `SESSION_COOKIE` |
| 路由处理器 | 业务 + 客户端/地区 | `tvplayurl`, `thseason_app` |
| 配置字段 | 地区_客户端_用途 | `cn_tv_playurl_api` |
| SQLite 表 | 复数 snake_case | `audit_events` |

避免继续引入错拼兼容字段；旧名称只应存在于迁移代码。

## 3. 错误边界

公共解析接口保持 Bilibili JSON 业务码；管理 API 使用真实 HTTP 状态和 `{ok,error}`。内部错误记录服务端上下文，客户端只得到稳定、非敏感消息。

Bad：对用户 JSON、数据库结果或上游字段直接 `unwrap()`。Good：验证输入，使用 `Option/Result/?`，在边界转换为对应响应。

## 4. 配置演进

```text
new field
  -> serde default
  -> version/migration
  -> validation
  -> JSON + YAML examples
  -> docs
  -> migration test
```

配置保存必须先反序列化到 `BiliConfig`，不能只验证 JSON 语法。秘密字段在管理响应中掩码；原子写入失败要恢复旧文件。

## 5. 异步与并发

- 网络和队列等待使用异步 API。
- 不跨 `.await` 持有同步锁。
- 后台并发必须限制排队量和已启动任务数；有界队列本身不能限制消费者继续 spawn 的任务。
- 主请求路径不等待审计、通知或标题补全。
- 修改共享运行配置时替换完整快照。

Bad：为每条审计先 `tokio::spawn` 再等待 Semaphore，等待任务仍可无限增长。Good：先 `try_acquire_owned`，取得许可后才 spawn。

现有 `BackgroundTaskType` 路径只把排队项限制为 120，生产者和消费者两侧仍会 spawn；这是待收敛的遗留行为，不应作为新模块范例。

## 6. 数据与安全

- access key 规则只保存 SHA-256 指纹。
- 日志不写 token、Cookie、签名 URL、appsec 或刷新响应。
- 只有可信 TCP 对端才能提供转发 IP 头。
- 除首次初始化和登录外，受保护的管理修改必须同时通过会话与 CSRF。
- 密码使用 Argon2，轮换后废除其他会话。
- SQLite 备份覆盖主文件/WAL/SHM，或停服复制整个状态目录。

## 7. 管理 UI

管理端是工作型界面：导航稳定、信息密集、无营销内容。沿用现有 CSS 变量、原生表单和 Lucide，不引入 CDN。图标按钮必须有 `title` 或 `aria-label`。

响应式要求：

- 375px 视口无页面级横向溢出。
- 宽表格在 `.table-wrap` 内滚动。
- 固定底部导航必须由页面 padding 留出空间。
- 文本不覆盖按钮、徽标或相邻字段。

## 8. 测试尺度

| 改动 | 最小测试 |
|---|---|
| 配置字段 | 默认值、迁移、错误输入 |
| 规则 | 规范化、优先级、scope、过期、删除外键 |
| 管理修改 | 401、403、成功路径、秘密往返 |
| 审计 | 业务码、筛选、保留、队列关闭/满 |
| 公共路由 | 精确路由到上游，不访问真实网络 |
| Shell | ShellCheck + `bash -n` |
| UI | 浏览器桌面/移动截图和 console |

## 9. Git 与提交

- 分支使用 `codex/` 前缀。
- 每个 commit 只有一个清晰职责。
- 使用 Conventional Commit，例如 `feat(audit): add request filtering`。
- 不提交 `config.json`、数据库、证书、工具链或用户 `test.py`。
- 不用破坏性 Git 命令覆盖工作树中的未知修改。

## 10. 新代码检查清单

- 模块职责能否一句话说明。
- 是否复用现有类型和服务。
- 是否有无界内存/任务增长。
- 是否会把秘密写入日志、响应或持久层。
- 是否兼容旧配置和已有路由。
- 错误边界是否符合公共 API/管理 API 的不同约定。
- 自动化测试和文档是否同步。
- `AGENTS.md` 的全部命令是否通过。
