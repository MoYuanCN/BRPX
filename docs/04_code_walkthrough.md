# 一条请求的代码旅程

> 本文从 `main()` 出发，沿一条 TV 播放请求走到响应和审计落库，把目录地图对应到真实执行顺序。

## 0. 读前地图

```text
main.rs route
   |
AuditTrail middleware
   |
handler.rs
   |-- get_user_info
   |-- AccessControlService::evaluate
   |-- cache lookup
   `-- upstream_res
             |
       build_response macros
             |
AuditTrail captures result
             |
       AuditService queue -> SQLite
```

## 1. 入口初始化

`src/main.rs:189` 的 `main()` 依次完成：

1. 创建 Tokio Runtime 和日志格式。
2. 执行 v1-v4 到 v5 的配置迁移。
3. 初始化 Redis Pool、后台任务队列和 `data/brpx.db`。
4. 创建 `AuditService`，启动审计 Worker，以及启动后立即执行一次、此后每小时执行的清理任务。
5. 创建 `AppState`，将配置快照、存储和服务放入 Actix App。
6. 根据 TLS 配置选择 HTTP、HTTPS 或 HTTP 跳转监听器。

路由是显式注册的，例如：

```rust
#[get("/pgc/player/api/playurltv")]
async fn tvplayurl(req: HttpRequest) -> impl Responder {
    handle_playurl_request(&req, true, false, true).await
}
```

最后一个 `true` 强制 TV 语义，避免再从 `fnval` 猜测。

## 2. 中间件先建立审计上下文

`AuditTrailMiddleware::call()` 从路径和查询串提取范围、地区、`ep_id`、`season_id`、关键词和可信客户端 IP。它不保存完整查询串，因此 access key 与签名不会进入审计。

上下文通过 Actix Request Extensions 附着到请求：

```text
HttpRequest.extensions
`-- AuditContext = Arc<Mutex<AuditEvent>>
```

Handler 可用 `update_context()` 补充 UID、缓存命中、上游和规则 ID。

## 3. Handler 解析和校验

`src/mods/handler.rs:26` 的 `handle_playurl_request()`：

1. 从 `AppState` 取得当前配置快照。
2. 解析地区、UA、appkey、签名、access key、`ep_id` 和客户端类型。
3. 调用 `get_user_info()` 获取 UID/VIP 状态。
4. 调用本地访问控制。

```rust
let access_decision = match state.access_control.evaluate(
    state.resolve_client_ip(req).into(),
    Some(user_info.uid),
    Some(params.access_key),
    if is_tv_route { "tv" } else { "playurl" },
) {
    Ok(value) => value,
    Err(_) => build_response!(EType::ServerGeneral),
};
```

拒绝规则立即返回兼容业务错误，允许规则可进入既有重签逻辑。

## 4. 缓存与上游

Handler 先调用 `get_cached_playurl()`。缓存命中直接返回；未命中则进入 `get_upstream_bili_playurl()`。

`ReqType::get_api()` 根据地区、APP/Web 和 TV 三个维度选择 URL：

```text
ReqType::Playurl(Cn, true, true)
             |
             `-> cn_tv_playurl_api
```

上游模块负责签名、代理、响应 JSON 修整、VIP/地区状态和 Redis 更新。Handler 只编排，不重复实现这些细节。

## 5. 响应与业务码

`build_response!` 和 `build_result_response!` 最终调用 `json_response()`：

1. 生成 Bilibili 兼容 JSON 与 CORS 头。
2. 解析 JSON 顶层 `code`。
3. 临时放入 `x-brpx-audit-business-code`。

审计中间件读取并删除该内部头，客户端只能看到原始兼容响应。

## 6. 审计异步落库

响应返回后，中间件写入 HTTP 状态和耗时，然后调用 `AuditService::record()`。该函数使用 `try_send`，不会等待 SQLite。

Worker 的后续步骤：

```text
insert audit_events
       |
       +-- no ep_id -> done
       |
       `-- ep_id -> try acquire one of 4 permits
                       |
                       `-> fetch season metadata -> upsert
```

管理页查询时 `audit_events` 左连接 `episode_metadata`，所以标题稍后出现不需要重写原事件。

## 7. 管理请求的不同路径

`/admin/api/*` 不进入公共审计。首次初始化状态、首次初始化和登录是公开入口；其他管理 API 校验 HttpOnly 会话，受保护的写操作还校验 `X-BRPX-CSRF`。配置保存采用掩码恢复、结构校验、原子替换和运行时快照更新。

## 8. 新模块最小结构

如果新增独立功能，遵循已有边界：

```text
src/mods/example.rs          数据和业务服务
src/mods.rs                  导出模块
src/main.rs                  只注册路由/启动 Worker
src/mods/handler.rs          需要时编排公共请求
src/html/admin.html          需要时增加管理入口
```

检查清单：路由是否有准确 scope；秘密是否只存在于请求期；失败是否使用兼容业务码；慢 I/O 是否离开主请求路径；测试是否覆盖路由到上游映射。
