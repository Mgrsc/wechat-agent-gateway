# wechat-agent-gateway

[中文说明](./README.zh-CN.md)

`wechat-agent-gateway` is an independent Rust gateway for the official WeChat bot channel.

It is derived from the behavior and protocol flow of Tencent's official packages:

- [`@tencent-weixin/openclaw-weixin-cli`](https://www.npmjs.com/package/@tencent-weixin/openclaw-weixin-cli)
- [`@tencent-weixin/openclaw-weixin`](https://www.npmjs.com/package/@tencent-weixin/openclaw-weixin)

This project rewrites that WeChat channel layer in Rust, removes the OpenClaw-specific coupling, and exposes a generic event/action gateway interface so other agent backends can integrate through a translator layer.

It does three things:

- talks to the official WeChat bot protocol
- normalizes inbound WeChat messages into standard events
- accepts standard outbound actions and sends them back to WeChat

It is not an agent, does not run model inference, and does not own backend business state.

## Features

- QR code login
- account persistence
- text send and receive
- typing
- inbound: `text / image / voice / file / video`
- outbound: `text / image / file / video`
- media upload, download, and decryption
- continues working after restart

Outbound `voice` is intentionally not implemented because the official package does not expose it as a standard outbound path.

## Environment

See [`.env.example`](/home/ian/dev/clones/wechat-openclaw/.env.example).

Most commonly used variables:

- `WECHAT_GATEWAY_BIND`
- `WECHAT_GATEWAY_PORT`
- `WECHAT_GATEWAY_URL`
- `WECHAT_GATEWAY_STATE_PATH`
- `WECHAT_GATEWAY_BASE_URL`
- `WECHAT_GATEWAY_CDN_BASE_URL`
- `RUST_LOG`

## Local Run

Start the server:

```bash
cargo run
```

Login:

```bash
cargo run -- login
```

## Docker

Pull and start:

```bash
docker compose up -d
```

Compose loads container runtime settings from [`.env.docker`](/home/ian/dev/clones/wechat-openclaw/.env.docker).

Login inside the container:

```bash
docker compose exec wechat-agent-gateway wechat-agent-gateway login
```

Published image:

- `ghcr.io/mgrsc/wechat-agent-gateway`

Release tags:

- `main`
- `latest`
- `vX.Y.Z`
- `X.Y.Z`
- `latest` also updates on `main`

## API

- `GET /v1/health`
- `GET /v1/accounts`
- `POST /v1/accounts/login/start`
- `GET /v1/accounts/login/{login_id}`
- `POST /v1/events/pull`
- `POST /v1/media`
- `GET /v1/media/{media_id}`
- `GET /v1/conversations/{conversation_id}`
- `POST /v1/actions`

For full interface details, deployment notes, and troubleshooting, see [AGENT_README.md](/home/ian/dev/clones/wechat-openclaw/AGENT_README.md).
