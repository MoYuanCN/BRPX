# 新功能接入指南

> 本文以“新增一种公共 API 能力”为主线，说明怎样在不破坏配置、规则、审计和兼容响应的前提下接入 BRPX。

## 1. 最小接入模型

一个公共功能通常涉及三层：

```text
route (main.rs)
   |
handler (validation/orchestration)
   |
service or upstream module
```

只有需要持久化时才增加 `storage.rs` 表或独立服务；只有需要管理时才增加 `/admin/api` 和 UI。

## 2. 先定义行为边界

开始编码前写清：

| 问题 | 示例答案 |
|---|---|
| 路径是什么 | `/pgc/player/api/playurltv` |
| 审计 scope | `tv` |
| 是否需要 UID/token | TV 播放需要用户信息 |
| 缓存是否与已有路由共享 | TV 使用独立语义 |
| 上游是否可配置 | 四地区分别配置 |
| 失败响应 | 保持 Bilibili JSON 业务码 |

## 3. 注册路由

TV 的真实写法位于 `src/main.rs`：

```rust
#[get("/pgc/player/api/playurltv")]
async fn tvplayurl(req: HttpRequest) -> impl Responder {
    handle_playurl_request(&req, true, false, true).await
}
```

三个 HTTP Server 分支必须注册同一服务。新增路由后也更新 `errorurl_reg()` 的兼容路径映射和路由测试。

## 4. 配置字段

如果能力需要配置：

1. 在 `BiliConfig` 增加字段。
2. 给新字段添加 `#[serde(default = "...")]`。
3. 必要时提高 `config_version`。
4. 在 `migrate_config_value()` 保留旧值并删除废弃字段。
5. 同步两个 example 与 `docs/configuration.md`。
6. 在管理端 `validate_config()` 验证 URL、范围和数值。

Bad：只改 JSON example，旧安装启动即失败。Good：结构体默认值、迁移、example、文档和测试同批提交。

## 5. 规则接入

公共 Handler 在取得足够身份信息后调用 `AccessControlService::evaluate()`，依次传入客户端 IP、可选 UID、可选 token 和 scope。不要照抄抽象参数名作为 Rust 代码；以 `handler.rs` 中各入口的真实调用为准。

当前各接口能够提供给规则引擎的身份并不相同：

| scope | IP/CIDR | UID | token |
|---|---|---|---|
| `playurl` / `tv` | 是 | 是 | 是 |
| `search` | 是 | 仅部分 APP 请求 | 请求带有效值时 |
| `season` | 是 | 否 | 是 |
| `subtitle` / `accesskey` | 是 | 否 | 否 |

新增 scope 时同时修改：

- `VALID_SCOPES`
- 管理 UI 下拉选项
- `docs/security.md`
- scope 匹配测试

不要在新模块中另写一套黑名单判断。

## 6. 审计接入

`AuditTrail` 会自动记录公共路径、查询中的剧集/地区和最终状态。Handler 只补充中间件不知道的字段：

```rust
update_context(req, |event| {
    event.uid = Some(user_info.uid);
    event.area = params.area.to_string();
    event.client_type = if is_tv_route {
        "tv"
    } else if is_app {
        "app"
    } else {
        "web"
    }
    .to_string();
    event.ep_id = params.ep_id.parse().ok();
    event.blocked = access_decision.denied;
    event.matched_rule_id = access_decision.rule_id;
});
```

禁止把完整查询、Cookie、Authorization、access key、签名 URL 填入事件。

## 7. 上游和缓存

上游选择应集中在 `ReqType` 或 `upstream_res.rs`。Handler 不应拼接一份重复 URL。缓存键必须区分响应语义；TV 与 APP 即使路径参数相似，也可能有不同字段和清晰度规则。

```text
new semantic?
  |-- no -> reuse existing CacheType
  `-- yes -> extend CacheType key deterministically
```

## 8. 管理 API 与 UI

首次初始化状态、首次初始化和登录必须保持公开，分别由初始化状态和凭据校验保护。其余只读 API 调用 `authenticate(..., false)`；受保护的修改调用 `authenticate(..., true)` 以要求 CSRF。秘密字段经 `redact_secrets()` 返回，保存时通过 `preserve_masked_values()` 恢复。

UI 没有构建步骤：使用现有 CSS 变量、5-6px 圆角、Lucide 图标和原生控件。新增页面必须在桌面和 375px 视口检查无横向页面溢出。

## 9. 完整测试清单

- 路由准确选择上游和客户端语义。
- 配置旧版本迁移且不覆盖正确字段。
- 规则覆盖优先级、scope、过期和规范化。
- 受保护的管理修改覆盖未登录与缺 CSRF，初始化/登录覆盖各自的公开边界。
- 审计覆盖业务码、筛选、保留期和队列失败。
- Shell 改动通过 ShellCheck 和 `bash -n`。
- 浏览器检查控制台、桌面和移动布局。

完成后运行 `AGENTS.md` 的全部命令，并用 Conventional Commit 按职责提交。
