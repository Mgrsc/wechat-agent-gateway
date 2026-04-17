# AGENT_README

## 1. Project Role

`wechat-agent-gateway` is an independent WeChat channel gateway.

Responsibilities:

- Speak to the official WeChat bot protocol
- Handle login, polling, message send, typing, and media upload/download
- Expose normalized inbound events and outbound actions to upstream systems

Out of scope:

- Model inference
- Agent orchestration
- Prompt management and long-term memory
- Backend business state

If this gateway is used with OpenClaw, Codex, Claude Code, or any other agent backend, the backend should provide its own translator layer.

## 2. Capability Scope

### Supported

- Login:
  - QR code login
  - login status polling
  - account persistence
- Inbound messages:
  - `text`
  - `image`
  - `voice`
  - `file`
  - `video`
- Outbound actions:
  - `typing`
  - `send_text`
  - `send_media(image)`
  - `send_media(file)`
  - `send_media(video)`
- Inbound media download and decryption
- Outbound media upload and send
- Minimal transport-state persistence

Inbound voice behavior:

- A voice event is always exposed as `kind = "voice"`
- If the upstream WeChat gateway includes speech-to-text in `voice_item.text`, the gateway copies it into:
  - `event.text`
  - `media[].transcript`
- If upstream does not include transcription, both values remain empty and only the audio media is exposed
- The gateway does not run ASR on its own

### Explicitly Not Supported

- Outbound `voice`
- Webhook / push callback mode
- OpenAI-compatible server mode
- Backend business-state persistence

## 3. Minimal Transport State

The gateway persists only:

- accounts
- `bot_token`
- `get_updates_buf`
- the latest conversation `context_token`

The gateway does not persist:

- event queues
- uploaded outbound media bytes
- agent conversation history
- inference context

## 4. Environment Variables

- `WECHAT_GATEWAY_BIND`
  - default: `127.0.0.1`
- `WECHAT_GATEWAY_PORT`
  - default: `8080`
- `WECHAT_GATEWAY_URL`
  - default: `http://127.0.0.1:<PORT>`
  - the `login` CLI uses this address by default
- `WECHAT_GATEWAY_STATE_PATH`
  - default: `$XDG_STATE_HOME/wechat-agent-gateway/state.json`
  - fallback: `~/.local/state/wechat-agent-gateway/state.json`
- `WECHAT_GATEWAY_BASE_URL`
  - default: `https://ilinkai.weixin.qq.com`
- `WECHAT_GATEWAY_CDN_BASE_URL`
  - default: `https://novac2c.cdn.weixin.qq.com`
- `RUST_LOG`
  - recommended: `wechat_agent_gateway=info,tower_http=info`

## 5. Deployment

### Local Run

```bash
cargo run
```

Login:

```bash
cargo run -- login
```

### Docker

Start:

```bash
docker compose up -d
```

The compose file loads container runtime settings from [`.env.docker`](/home/ian/dev/clones/wechat-openclaw/.env.docker).

Login:

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

## 6. Standard Interfaces

### 6.1 Health Check

`GET /v1/health`

Response:

```json
{
  "status": "ok",
  "version": "0.1.0",
  "account_count": 1
}
```

### 6.2 List Accounts

`GET /v1/accounts`

Response:

```json
{
  "accounts": [
    {
      "account_id": "<account_id>",
      "label": "default",
      "enabled": true,
      "configured": true,
      "base_url": "https://ilinkai.weixin.qq.com"
    }
  ]
}
```

### 6.3 Start Login

`POST /v1/accounts/login/start`

Request:

```json
{
  "label": "default"
}
```

Response:

```json
{
  "login_id": "login_1",
  "qrcode_url": "https://liteapp.weixin.qq.com/q/...",
  "status": "waiting"
}
```

### 6.4 Get Login Status

`GET /v1/accounts/login/{login_id}`

Successful response:

```json
{
  "login_id": "login_1",
  "status": "confirmed",
  "account_id": "<account_id>"
}
```

### 6.5 Pull Events

`POST /v1/events/pull`

Request:

```json
{
  "cursor": null,
  "account_id": "<account_id>",
  "wait_ms": 30000,
  "limit": 50
}
```

Field semantics:

- `cursor`
  - gateway-level event cursor
  - not the raw WeChat `get_updates_buf`
- `wait_ms`
  - intended maximum wait time for the gateway pull request
  - current implementation does not enforce this as a strict upper bound
  - if the local queue is empty, the gateway immediately performs one upstream WeChat `getupdates` call
  - the actual blocking time is therefore mainly determined by upstream WeChat long-poll behavior
- `limit`
  - maximum number of normalized gateway events returned in a single `/v1/events/pull` response
  - this does not directly limit how many raw messages the upstream WeChat `getupdates` call may return
  - larger values improve per-response throughput
  - smaller values reduce per-batch processing size

Response example:

```json
{
  "events": [
    {
      "event_id": "evt_7441673000628506888",
      "conversation_id": "wechat:<account_id>:<peer_id>",
      "channel": "wechat",
      "account_id": "<account_id>",
      "peer_id": "<peer_id>",
      "message_id": "7441673000628506888",
      "context_token": "opaque",
      "kind": "text",
      "text": "hi",
      "quoted_text": null,
      "media": [],
      "timestamp_ms": 1774233102899,
      "raw_ref": {
        "item_type": 1,
        "platform": "wechat",
        "seq": 1
      }
    }
  ],
  "next_cursor": "1"
}
```

### 6.6 Upload Media

`POST /v1/media`

`multipart/form-data` fields:

- `account_id`
- `kind`
  - `image`
  - `file`
  - `video`
- `file`

Response:

```json
{
  "media_id": "med_out_xxx",
  "kind": "image",
  "size": 62040,
  "status": "ready"
}
```

### 6.7 Download Media

`GET /v1/media/{media_id}`

Behavior:

- If the media was uploaded by the current process as outbound media, the gateway returns the original bytes directly.
- If the media came from an inbound WeChat message, the gateway downloads it from the CDN, decrypts it, and then returns the plaintext bytes.

### 6.8 Get Conversation

`GET /v1/conversations/{conversation_id}`

Used to inspect the minimal conversation state currently tracked by the gateway.

### 6.9 Send Actions

`POST /v1/actions`

Request:

```json
{
  "conversation_id": "wechat:<account_id>:<peer_id>",
  "context_token": "opaque",
  "actions": [
    {
      "type": "typing",
      "status": "start"
    },
    {
      "type": "send_text",
      "text": "hello from gateway"
    }
  ]
}
```

Supported action types:

- `typing`
- `send_text`
- `send_media`

`send_media` example:

```json
{
  "conversation_id": "wechat:...",
  "context_token": "opaque",
  "actions": [
    {
      "type": "send_media",
      "media_id": "med_out_xxx",
      "caption": "test media"
    }
  ]
}
```

## 7. Integration Guidance

Recommended flow:

1. The backend polls `/v1/events/pull`
2. It translates the normalized inbound event into its own internal input format
3. The backend produces normalized outbound actions
4. It calls `/v1/actions` to send the result back to WeChat

Do not expose the raw WeChat protocol directly to upper layers.

## 8. Troubleshooting

### Login State Lost After Restart

Check:

- whether `WECHAT_GATEWAY_STATE_PATH` is fixed
- whether the container mounts `/data`

### `context_token_missing`

Cause:

- `/v1/actions` was called without `context_token`

Fix:

- read the latest `context_token` from `/v1/events/pull`

### `missing field conversation_id`

Cause:

- the `/v1/actions` request body does not include `conversation_id`

### `invalid_media_upload`

Common causes:

- incorrect `multipart/form-data` field names
- unknown `account_id`
- unsupported `kind`
- empty uploaded file
- request body too large while still running an old binary

The current upload limit is `64 MiB`.

### Image Appears as Expired

This usually means the `aes_key` encoding is wrong. The current implementation follows the official protocol:

- keep `aeskey` as a hex string first
- base64-encode that hex string when building `sendmessage`

### Video or File Send Fails

Check:

- whether `/v1/media` returned a valid `media_id`
- whether `/v1/actions` includes the correct `conversation_id`
- whether `/v1/actions` includes the correct `context_token`

### Can It Still Work After Restart?

Yes, as long as:

- the account was logged in successfully
- the state file still exists

Notes:

- outbound media uploaded in memory is not preserved across restarts
- outbound `media_id` values are not meant to be reused after restart

## 9. Current Implementation Boundary

This implementation matches the official WeChat channel capability set, but it does not include:

- OpenClaw plugin registration
- OpenClaw route/session/reply dispatcher
- OpenClaw slash commands
- OpenAI-compatible API mode
- outbound `voice`
