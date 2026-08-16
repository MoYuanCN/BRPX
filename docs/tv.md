# TV 端接入

## 接口

BRPX 显式处理以下 TV 播放接口：

```text
Host: api.snm0516.aisee.tv
Path: /pgc/player/api/playurltv
Upstream: https://api.snm0516.aisee.tv/pgc/player/api/playurltv
```

路由命中后会强制使用 TV 语义和独立缓存键，不依赖 `fnval` 推断客户端。四个地区可以分别修改 `cn_tv_playurl_api`、`hk_tv_playurl_api`、`tw_tv_playurl_api` 和 `th_tv_playurl_api`。

## 客户端流量指向

优先使用支持自定义解析服务器的 TV 客户端或模块，将 TV API 基址改为 BRPX 的 HTTPS 地址。

如果只能通过 DNS/Hosts 把 `api.snm0516.aisee.tv` 指向 BRPX，HTTPS 证书必须被 TV 设备信任且覆盖该域名。公共 ACME 服务不会为不属于你的域名签发证书；这类部署需要设备侧受信任的本地 CA，并可能受证书固定限制。仅修改 DNS 而不处理 TLS 不会正常工作。

## 反向代理

解析域名由反向代理终止 TLS 时，应保留原始 Host 和查询串：

```nginx
server {
    listen 443 ssl http2;
    server_name api.snm0516.aisee.tv;

    location = /pgc/player/api/playurltv {
        proxy_pass http://127.0.0.1:2662;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
}
```

## 验证

使用脱敏后的真实 TV 查询参数验证，不要在命令历史中保留有效 `access_key`。开发测试已经覆盖 `/pgc/player/api/playurltv` 到独立 TV 上游的选择；生产联调还应确认具体 appkey、签名、清晰度参数和设备端响应兼容性。

域名和路径应通过脱敏实测请求复核；外部测试脚本不是生产组件。
