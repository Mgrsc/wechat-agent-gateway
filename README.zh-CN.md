# wechat-agent-gateway

[English README](./README.md)

`wechat-agent-gateway` 是一个独立的 Rust 微信网关，面向微信官方 bot 通道。

它只做三件事：

- 跟微信官方协议通讯
- 把微信入站消息标准化为统一事件
- 接收统一动作并回发到微信

它不是 Agent，不做模型推理，也不负责后端业务状态。

## 功能

- 二维码登录
- 账号持久化
- 文本收发
- typing
- 入站：`text / image / voice / file / video`
- 出站：`text / image / file / video`
- 媒体上传、下载和解密
- 重启后继续工作

主动发送 `voice` 没做，因为官方包本身没有把它做成标准出站主路径。

## 环境变量

见 [`.env.example`](/home/ian/dev/clones/wechat-openclaw/.env.example)。

常用变量：

- `WECHAT_GATEWAY_BIND`
- `WECHAT_GATEWAY_PORT`
- `WECHAT_GATEWAY_URL`
- `WECHAT_GATEWAY_STATE_PATH`
- `WECHAT_GATEWAY_BASE_URL`
- `WECHAT_GATEWAY_CDN_BASE_URL`
- `RUST_LOG`

## 本地运行

启动服务：

```bash
cargo run
```

登录：

```bash
cargo run -- login
```

## Docker

构建并启动：

```bash
docker compose up --build -d
```

在容器内登录：

```bash
docker compose exec wechat-agent-gateway wechat-agent-gateway login
```

发布镜像：

- `ghcr.io/mgrsc/wechat-agent-gateway`

发布标签：

- `main`
- `vX.Y.Z`
- `X.Y.Z`
- 版本 tag 同步推 `latest`

## 接口

- `GET /v1/health`
- `GET /v1/accounts`
- `POST /v1/accounts/login/start`
- `GET /v1/accounts/login/{login_id}`
- `POST /v1/events/pull`
- `POST /v1/media`
- `GET /v1/media/{media_id}`
- `GET /v1/conversations/{conversation_id}`
- `POST /v1/actions`

完整接口、部署说明和排障说明见 [AGENT_README.md](/home/ian/dev/clones/wechat-openclaw/AGENT_README.md)。
