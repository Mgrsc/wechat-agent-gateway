use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tracing::info;

use crate::config::RuntimeConfig;
use crate::errors::AppError;
use crate::logging::{redact_id, sanitize_url};
use crate::media::build_outbound_media_asset;
use crate::models::{
    AccountsResponse, ActionBatch, ActionBatchResponse, ActionResult, EventKind, HealthResponse,
    LoginStartRequest, LoginStartResponse, LoginStatusResponse, MediaUploadResponse,
    OutboundAction, PullEventsRequest, PullEventsResponse,
};
use crate::state::AppState;
use crate::wechat::normalize_message;
use crate::wechat_client::WechatClient;

#[derive(Debug, Clone)]
pub struct AppContext {
    pub state: AppState,
    pub config: RuntimeConfig,
}

pub fn build_router(state: AppState) -> Router {
    build_router_with_config(state, RuntimeConfig::default())
}

pub fn build_router_with_config(state: AppState, config: RuntimeConfig) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/accounts", get(accounts))
        .route("/v1/accounts/login/start", post(login_start))
        .route("/v1/accounts/login/{login_id}", get(login_status))
        .route("/v1/events/pull", post(pull_events))
        .route("/v1/media", post(upload_media))
        .route("/v1/media/{media_id}", get(download_media))
        .route("/v1/conversations/{conversation_id}", get(get_conversation))
        .route("/v1/actions", post(apply_actions))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(AppContext { state, config })
}

async fn health(State(context): State<AppContext>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        account_count: context.state.account_count(),
    })
}

async fn accounts(State(context): State<AppContext>) -> Json<AccountsResponse> {
    Json(AccountsResponse {
        accounts: context.state.list_accounts(),
    })
}

async fn get_conversation(
    Path(conversation_id): Path<String>,
    State(context): State<AppContext>,
) -> Result<Json<crate::models::Conversation>, AppError> {
    Ok(Json(context.state.get_conversation(&conversation_id)?))
}

async fn login_start(
    State(context): State<AppContext>,
    Json(request): Json<LoginStartRequest>,
) -> Result<Json<LoginStartResponse>, AppError> {
    let base_url = request
        .base_url
        .clone()
        .unwrap_or_else(|| context.config.wechat_base_url.clone());
    let client = WechatClient::with_cdn(
        base_url.clone(),
        Some(context.config.wechat_cdn_base_url.clone()),
    );
    let payload = client.start_login().await?;
    let session = context
        .state
        .create_login_session(request.label, base_url, payload.qrcode);

    info!(
        login_id = %session.login_id,
        qrcode_url = %sanitize_url(&payload.qrcode_url),
        "login session started"
    );

    Ok(Json(LoginStartResponse {
        login_id: session.login_id,
        qrcode_url: payload.qrcode_url,
        status: session.status,
    }))
}

async fn login_status(
    Path(login_id): Path<String>,
    State(context): State<AppContext>,
) -> Result<Json<LoginStatusResponse>, AppError> {
    let session = context.state.get_login_session(&login_id)?;
    let client = WechatClient::with_cdn(
        session.base_url.clone(),
        Some(context.config.wechat_cdn_base_url.clone()),
    );
    let payload = client.get_login_status(&session.qrcode).await?;
    let session = context
        .state
        .update_login_session_status(&login_id, &payload)?;

    Ok(Json(LoginStatusResponse {
        login_id: session.login_id,
        status: session.status,
        account_id: session.account_id,
    }))
}

async fn pull_events(
    State(context): State<AppContext>,
    Json(request): Json<PullEventsRequest>,
) -> Result<Json<PullEventsResponse>, AppError> {
    let (mut events, mut next_cursor) = context.state.pull_events(
        &request.account_id,
        request.cursor.as_deref(),
        request.limit,
    )?;

    if events.is_empty() {
        let account = context.state.get_account_session(&request.account_id)?;
        let client = WechatClient::with_cdn(
            account.base_url,
            Some(context.config.wechat_cdn_base_url.clone()),
        );
        let updates = client
            .get_updates(&account.bot_token, account.wechat_cursor.as_deref())
            .await?;

        if let Some(wechat_cursor) = updates.get_updates_buf {
            context
                .state
                .update_account_cursor(&account.account_id, wechat_cursor)?;
        }

        for message in updates.msgs {
            let event = normalize_message(&message)
                .map_err(|error| AppError::Normalization(error.to_string()))?;
            context.state.enqueue_event(event)?;
        }

        (events, next_cursor) = context.state.pull_events(
            &request.account_id,
            request.cursor.as_deref(),
            request.limit,
        )?;
    }

    info!(
        account_id = %redact_id(&request.account_id),
        wait_ms = request.wait_ms,
        limit = request.limit,
        returned = events.len(),
        "events pull request handled"
    );

    Ok(Json(PullEventsResponse {
        events,
        next_cursor,
    }))
}

async fn apply_actions(
    State(context): State<AppContext>,
    Json(request): Json<ActionBatch>,
) -> Result<Json<ActionBatchResponse>, AppError> {
    validate_actions(&context.state, &request)?;
    let response = if request.actions.iter().all(|action| {
        matches!(
            action,
            OutboundAction::Typing { .. }
                | OutboundAction::SendText { .. }
                | OutboundAction::SendMedia { .. }
        )
    }) {
        match context
            .state
            .get_dispatch_context(&request.conversation_id)?
        {
            Some(dispatch) => {
                let client = WechatClient::with_cdn(
                    dispatch.base_url.clone(),
                    Some(context.config.wechat_cdn_base_url.clone()),
                );
                let mut results = Vec::with_capacity(request.actions.len());

                for action in &request.actions {
                    match action {
                        OutboundAction::Typing { status } => {
                            let context_token = request
                                .context_token
                                .as_deref()
                                .ok_or(AppError::ContextTokenMissing)?;
                            let config = client
                                .get_config(&dispatch.bot_token, &dispatch.peer_id, context_token)
                                .await?;
                            client
                                .send_typing(
                                    &dispatch.bot_token,
                                    &dispatch.peer_id,
                                    &config.typing_ticket,
                                    status.clone(),
                                )
                                .await?;
                            results.push(ActionResult {
                                action_type: "typing",
                                ok: true,
                                message_id: None,
                            });
                        }
                        OutboundAction::SendText { text } => {
                            let context_token = request
                                .context_token
                                .as_deref()
                                .ok_or(AppError::ContextTokenMissing)?;
                            let message_id = client
                                .send_text(
                                    &dispatch.bot_token,
                                    &dispatch.peer_id,
                                    context_token,
                                    text,
                                )
                                .await?;
                            results.push(ActionResult {
                                action_type: "send_text",
                                ok: true,
                                message_id: Some(message_id),
                            });
                        }
                        OutboundAction::SendMedia { media_id, caption } => {
                            let context_token = request
                                .context_token
                                .as_deref()
                                .ok_or(AppError::ContextTokenMissing)?;
                            let asset = context
                                .state
                                .get_media_asset(media_id)
                                .ok_or(AppError::MediaNotFound)?;

                            match asset.descriptor.kind {
                                EventKind::Image => {
                                    let message_id = client
                                        .send_image(
                                            &dispatch.bot_token,
                                            &dispatch.peer_id,
                                            context_token,
                                            caption.as_deref(),
                                            &client
                                                .upload_image(
                                                    &dispatch.bot_token,
                                                    &dispatch.peer_id,
                                                    &asset.bytes,
                                                )
                                                .await?,
                                        )
                                        .await?;

                                    results.push(ActionResult {
                                        action_type: "send_media",
                                        ok: true,
                                        message_id: Some(message_id),
                                    });
                                }
                                EventKind::Video => {
                                    let uploaded = client
                                        .upload_video(
                                            &dispatch.bot_token,
                                            &dispatch.peer_id,
                                            &asset.bytes,
                                        )
                                        .await?;
                                    let message_id = client
                                        .send_video(
                                            &dispatch.bot_token,
                                            &dispatch.peer_id,
                                            context_token,
                                            caption.as_deref(),
                                            &uploaded,
                                        )
                                        .await?;

                                    results.push(ActionResult {
                                        action_type: "send_media",
                                        ok: true,
                                        message_id: Some(message_id),
                                    });
                                }
                                EventKind::File => {
                                    let uploaded = client
                                        .upload_file(
                                            &dispatch.bot_token,
                                            &dispatch.peer_id,
                                            &asset.bytes,
                                        )
                                        .await?;
                                    let file_name = asset
                                        .descriptor
                                        .filename
                                        .as_deref()
                                        .ok_or(AppError::InvalidAction)?;
                                    let message_id = client
                                        .send_file(
                                            &dispatch.bot_token,
                                            &dispatch.peer_id,
                                            context_token,
                                            caption.as_deref(),
                                            file_name,
                                            &uploaded,
                                        )
                                        .await?;

                                    results.push(ActionResult {
                                        action_type: "send_media",
                                        ok: true,
                                        message_id: Some(message_id),
                                    });
                                }
                                _ => return Err(AppError::InvalidAction),
                            }
                        }
                    }
                }

                context
                    .state
                    .mark_outbound(&request.conversation_id, request.context_token.as_deref())?;
                ActionBatchResponse { ok: true, results }
            }
            None => context.state.apply_actions(&request)?,
        }
    } else {
        context.state.apply_actions(&request)?
    };
    Ok(Json(response))
}

async fn upload_media(
    State(context): State<AppContext>,
    mut multipart: Multipart,
) -> Result<Json<MediaUploadResponse>, AppError> {
    let mut account_id = None;
    let mut kind = None;
    let mut filename = None;
    let mut mime = None;
    let mut file_bytes = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::InvalidMediaUpload(error.to_string()))?
    {
        let name = field.name().map(str::to_owned).unwrap_or_default();
        match name.as_str() {
            "account_id" => {
                account_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| AppError::InvalidMediaUpload(error.to_string()))?,
                );
            }
            "kind" => {
                kind = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| AppError::InvalidMediaUpload(error.to_string()))?,
                );
            }
            "file" => {
                filename = field.file_name().map(str::to_owned);
                mime = field.content_type().map(str::to_owned);
                file_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|error| AppError::InvalidMediaUpload(error.to_string()))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    let account_id =
        account_id.ok_or_else(|| AppError::InvalidMediaUpload("account_id is required".into()))?;
    let kind = match kind.as_deref() {
        Some("image") => EventKind::Image,
        Some("video") => EventKind::Video,
        Some("file") => EventKind::File,
        Some(other) => {
            return Err(AppError::InvalidMediaUpload(format!(
                "unsupported media kind {other}"
            )));
        }
        None => return Err(AppError::InvalidMediaUpload("kind is required".into())),
    };
    let bytes =
        file_bytes.ok_or_else(|| AppError::InvalidMediaUpload("file is required".into()))?;

    if context.state.get_account_session(&account_id).is_err() {
        return Err(AppError::AccountNotFound);
    }

    let asset = build_outbound_media_asset(account_id, kind, filename, mime, bytes)?;
    let descriptor = context.state.insert_media_asset(asset);

    Ok(Json(MediaUploadResponse {
        media_id: descriptor.media_id,
        kind: descriptor.kind,
        size: descriptor.size.unwrap_or_default(),
        status: "ready",
    }))
}

async fn download_media(
    Path(media_id): Path<String>,
    State(context): State<AppContext>,
) -> Result<impl IntoResponse, AppError> {
    if let Some(asset) = context.state.get_media_asset(&media_id) {
        return Ok(build_media_response(
            asset.descriptor.mime.as_deref(),
            &asset.descriptor.kind,
            asset.descriptor.filename.as_deref(),
            asset.bytes,
        ));
    }

    let descriptor = context
        .state
        .get_media(&media_id)
        .ok_or(AppError::MediaNotFound)?;
    let account_id = context
        .state
        .get_media_account_id(&media_id)
        .ok_or(AppError::MediaNotFound)?;
    let account = context.state.get_account_session(&account_id)?;
    let client = WechatClient::with_cdn(
        account.base_url,
        Some(context.config.wechat_cdn_base_url.clone()),
    );
    let bytes = client.download_media(&descriptor).await?;

    Ok(build_media_response(
        descriptor.mime.as_deref(),
        &descriptor.kind,
        descriptor.filename.as_deref(),
        bytes,
    ))
}

fn build_media_response(
    mime: Option<&str>,
    kind: &EventKind,
    filename: Option<&str>,
    bytes: Vec<u8>,
) -> Response<Body> {
    let content_type = mime
        .map(str::to_owned)
        .unwrap_or_else(|| guess_content_type(kind, filename));
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response
}

fn guess_content_type(kind: &EventKind, filename: Option<&str>) -> String {
    if let Some(filename) = filename
        && let Some(ext) = filename.rsplit('.').next()
    {
        return match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "txt" => "text/plain; charset=utf-8",
            "pdf" => "application/pdf",
            "mp4" => "video/mp4",
            "mp3" => "audio/mpeg",
            "ogg" => "audio/ogg",
            _ => "application/octet-stream",
        }
        .to_string();
    }

    match kind {
        EventKind::Image => "image/png".to_string(),
        EventKind::Voice => "audio/ogg".to_string(),
        EventKind::Video => "video/mp4".to_string(),
        EventKind::File | EventKind::Text => "application/octet-stream".to_string(),
    }
}

fn validate_actions(state: &AppState, request: &ActionBatch) -> Result<(), AppError> {
    for action in &request.actions {
        match action {
            crate::models::OutboundAction::Typing { .. }
            | crate::models::OutboundAction::SendText { .. } => {
                if request.context_token.is_none() {
                    return Err(AppError::ContextTokenMissing);
                }
            }
            crate::models::OutboundAction::SendMedia { media_id, .. } => {
                if request.context_token.is_none() {
                    return Err(AppError::ContextTokenMissing);
                }
                if !state.has_media_asset(media_id) {
                    return Err(AppError::MediaNotFound);
                }
            }
        }
    }

    Ok(())
}
