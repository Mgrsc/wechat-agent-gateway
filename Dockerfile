FROM rust:1-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

LABEL org.opencontainers.image.source="https://github.com/Mgrsc/wechat-agent-gateway"
LABEL org.opencontainers.image.description="Independent Rust WeChat gateway for the official bot channel"

COPY --from=builder /app/target/release/wechat-agent-gateway /usr/local/bin/wechat-agent-gateway

RUN useradd --system --create-home --uid 10001 gateway \
    && mkdir -p /data \
    && chown -R gateway:gateway /data /app

USER gateway

ENV WECHAT_GATEWAY_BIND=0.0.0.0
ENV WECHAT_GATEWAY_PORT=8080
ENV WECHAT_GATEWAY_URL=http://127.0.0.1:8080
ENV WECHAT_GATEWAY_STATE_PATH=/data/state.json
ENV WECHAT_GATEWAY_BASE_URL=https://ilinkai.weixin.qq.com
ENV WECHAT_GATEWAY_CDN_BASE_URL=https://novac2c.cdn.weixin.qq.com

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/wechat-agent-gateway"]
