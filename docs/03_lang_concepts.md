# 阅读 BRPX 需要的 Rust 特性

> 本文只解释 BRPX 真实使用的 Rust 概念，目标是让有基础编程经验的新人能继续阅读代码。

## 1. `async` / `await`

项目位置：`src/mods/handler.rs:26` 的 `handle_playurl_request()`，以及 `src/mods/audit.rs:240` 的 `run_worker()`。

异步函数遇到网络、队列等待时把执行权交回 Tokio，而不是占住一个 OS 线程：

实际循环的核心条件是 `while let Ok(event) = receiver.recv().await`：通道关闭时循环结束，收到事件后再同步落库并尝试启动元数据补全。这里是执行顺序说明，不是可直接替换源码的简化实现。

没有异步时，一个慢上游会长期占用线程；使用异步后，同一运行时可调度其他请求。要点：不能在异步主路径随意执行长时间同步 I/O。

## 2. `Arc` 与共享所有权

项目位置：`src/mods/management.rs:71` 的 `AppState` 初始化，`src/mods/types.rs:153` 的 `BiliRuntime`。

`Arc<T>` 是线程安全引用计数。多个 Actix Worker 需要共享配置快照、Sender 和数据库包装对象，因此不能使用单一所有者。

```text
Arc<AppState>
  |-- worker 1
  |-- worker 2
  `-- background task
```

`Arc` 只解决所有权，不自动让内部数据可变。可变配置另用 `RwLock`，SQLite 连接另用 `Mutex`。

## 3. `RwLock<Arc<T>>` 快照

项目位置：`src/mods/management.rs` 的 `config_snapshot()`。

```rust
pub fn config_snapshot(&self) -> Arc<BiliConfig> {
    self.config
        .read()
        .expect("runtime config lock poisoned")
        .clone()
}
```

读锁只用于克隆内层 `Arc`，不会贯穿整个请求。与“直接给每个字段加锁”相比，这种写法保证一次请求看到同一份配置。

## 4. `Result`、`Option` 与 `?`

项目位置：`src/mods/storage.rs:14`、`src/mods/user_info.rs` 的令牌刷新。

| 类型 | 表达 |
|---|---|
| `Option<T>` | 可能没有值，但不一定是错误 |
| `Result<T, E>` | 成功值或明确错误 |
| `?` | 当前函数无法处理时提前返回 `None`/`Err` |

Bad：对外部 JSON 使用 `unwrap()`，字段缺失会 panic。Good：`as_str()?` 把异常变成当前函数的空结果。

## 5. Enum 与模式匹配

项目位置：`src/mods/types.rs` 的 `ReqType`、`Area`、`EType`。

`ReqType::Playurl(Area, is_app, is_tv)` 把请求类型和必要上下文绑定在一个值里。`match` 要覆盖所有变体，因此新增 TV 语义时编译器会指出还未更新的 API/代理选择位置。

## 6. Serde 派生与默认值

项目位置：`src/mods/types.rs` 的 `BiliConfig`，`src/mods/access_control.rs` 的 `AccessRuleInput`。

```rust
#[serde(default = "config_version")]
pub config_version: u16,
```

`derive` 自动生成 JSON/YAML 转换。新配置字段必须有 `serde(default)`，否则旧配置反序列化会失败。

## 7. Trait 驱动的中间件

项目位置：`src/mods/middleware/audit.rs`。

`Transform` 创建中间件实例，`Service` 包装下游请求。`call()` 在调用下游前记录开始时间，在响应返回后补 HTTP/业务码和耗时。

```text
Transform -> AuditTrailMiddleware<Service>
                      |
                      `-> call(next) -> inspect response
```

## 8. 宏

项目位置：`src/mods/types.rs:793` 的 `build_response!`。

宏统一 Bilibili 风格 JSON 响应和 CORS 头，并把业务码放入只供审计中间件读取的内部头。与普通函数相比，宏能在调用处直接 `return`，但也更难调试；新代码优先使用函数，只有需要这种控制流时才扩展宏。

## 9. 生命周期

项目位置：`BiliRuntime<'bili_runtime>`。

生命周期声明说明 `BiliRuntime` 只借用配置、Redis Pool、Sender 和访问控制服务，不能比这些对象活得更久。它避免每次业务调用都克隆重量级对象。

## 速查表

| 看到的写法 | 先想到 |
|---|---|
| `Arc<T>` | 跨任务共享所有权 |
| `Mutex<T>` | 单写者同步访问 |
| `RwLock<Arc<T>>` | 可替换的只读快照 |
| `.await` | 可能让出当前任务 |
| `?` | 错误/空值向上传播 |
| `match` | 按 Enum/模式穷举分支 |
| `#[serde(default)]` | 兼容旧配置 |
| `#[cfg(test)]` | 仅测试构建 |

检查清单：阅读异步代码时先标出借用对象的生命周期，再确认同步锁没有跨 `.await` 持有。
