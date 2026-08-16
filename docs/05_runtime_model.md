# BRPX 运行时模型

> 本文回答进程启动后有哪些执行单元、共享什么状态，以及请求与后台任务如何交互。

## 1. 进程全景

BRPX 是单进程、Actix Worker 与一个手工 Tokio Runtime 并存的异步模型。

```text
BRPX process
|-- main-created Tokio runtime
|   |-- background task receiver and spawned tasks
|   |-- audit writer
|   |-- audit prune timer
|   `-- up to 4 episode metadata fetches
|-- Actix HttpServer runtime(s)
|   |-- primary listener workers (worker_num)
|   `-- redirect listener workers (worker_num, only HTTPS+redirect)
|-- Redis connection pool
`-- SQLite connection behind Mutex (WAL mode)
```

`HttpServer` 自己创建 Worker 线程；手工 Tokio Runtime 承载业务队列、审计和清理任务。`worker_num` 是每个 `HttpServer` 的 Worker 数：普通 HTTP 或 HTTPS 模式为 N，HTTPS 加 HTTP 跳转模式会创建两个 Server，配置上限可达到 2N。它是启动期配置，修改后必须重启。

## 2. HTTP Worker

Actix 的 `.wrap()` 按逆注册顺序进入。公共请求的入站顺序是 `ChangeCompressPriority -> Compress -> AuditTrail -> Governor -> Handler`，响应按反方向返回。Handler 内网络访问是异步的，但 SQLite 小查询通过一个进程内 Mutex 串行执行，并会占用当前 Actix Worker。

注意：不要在持有 `Database` Mutex、`RwLock` Guard 或审计事件 Mutex 时 `.await`。当前封装让数据库闭包同步完成后再返回，配置快照也只在短读锁内克隆 `Arc`。

## 3. 两条有界队列

| 队列 | 容量 | 生产者 | 消费者 | 满时行为 |
|---|---:|---|---|---|
| `BackgroundTaskType` | 120 个排队项 | 缓存/健康逻辑 | `web_background` | 打印错误并丢弃 |
| `AuditEvent` | 2048 | `AuditTrail` | `run_audit_worker` | 丢弃并增加计数 |

审计队列限制待写事件，元数据补全还用 Semaphore 把实际请求限制为 4 个。业务队列的 120 只限制排队项：生产者在 `try_send` 前 spawn，消费者收到后也立即 spawn，因此它不限制已经启动的任务总数，不能视为完整背压。

## 4. 配置生命周期

```text
startup file -> BiliConfig -> Arc snapshot v1
                                |
admin save -> validate -> atomic file -> Arc snapshot v2
```

已开始的请求持有 v1 直到结束，新请求取得 v2。Redis Pool、监听器、Worker 数和速率限制器在启动时创建，因此相关修改显示为“需要重启”。

## 5. SQLite 生命周期

`Database::open()`：

1. 创建父目录。
2. 打开 `data/brpx.db`。
3. 设置 5 秒 busy timeout。
4. 启用 WAL 和外键。
5. 执行幂等 `CREATE TABLE IF NOT EXISTS`。

同一 `Connection` 由 Mutex 保护。WAL 允许外部备份工具更灵活地读，但直接复制单个主文件可能缺数据，所以运维备份应停服并复制整个状态目录。

## 6. 审计剧集补全

审计 Writer 先持久化事件，再用 `try_acquire_owned()` 获取四个并发许可。没有许可时跳过本次补全；同一剧集后续请求仍可重试。这样不会产生大量等待中的 Tokio Task。

## 7. 启动顺序

```text
logger
  -> config migration/load
  -> Redis Pool and channels
  -> SQLite schema
  -> AppState
  -> audit worker/prune timer (first prune immediately)
  -> initial resign cache preparation
  -> Actix listener(s)
```

启动前没有配置文件时进程以退出码 78 结束。Redis 暂时不可用时管理仪表盘显示异常，具体业务是否可降级取决于请求路径。

## 8. 关闭与重启

systemd 使用 `Restart=on-failure`。当前 Ctrl-C/终止信号处理器直接调用 `std::process::exit(0)`，不会等待 Actix 请求或后台队列排空，因此不能称为优雅关闭。安装器升级时先停止服务，再备份 SQLite 主文件和可能存在的 WAL/SHM，随后替换二进制；重要备份应在停服完成后进行。

## 速查表

| 问题 | 答案 |
|---|---|
| Handler 在哪里执行 | 某个 Actix Worker 的异步任务 |
| 配置保存会中断旧请求吗 | 不会，旧请求保留旧 `Arc` |
| 审计会阻塞播放吗 | 队列入队不等待；满时丢弃 |
| 剧集补全并发 | 最多 4 个实际请求 |
| SQLite 是否每请求新连接 | 否，共享单连接并由 Mutex 保护 |
| 哪些修改必须重启 | 端口/TLS/Worker/Redis/速率限制 |

检查清单：观察队列容量、锁持有范围和启动期对象；把“异步函数”与“其中所有操作都非阻塞”区分开。
