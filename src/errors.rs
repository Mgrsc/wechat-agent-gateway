use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("account not found")]
    AccountNotFound,
    #[error("login session not found")]
    LoginNotFound,
    #[error("conversation not found")]
    ConversationNotFound,
    #[error("context token missing")]
    ContextTokenMissing,
    #[error("media not found")]
    MediaNotFound,
    #[error("invalid cursor")]
    InvalidCursor,
    #[error("invalid action")]
    InvalidAction,
    #[error("invalid media upload: {0}")]
    InvalidMediaUpload(String),
    #[error("wechat api error: {0}")]
    WechatApi(String),
    #[error("normalization failed: {0}")]
    Normalization(String),
    #[error("state storage failed: {0}")]
    StateStorage(String),
}

#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    fn code(&self) -> &'static str {
        match self {
            Self::AccountNotFound => "account_not_found",
            Self::LoginNotFound => "login_not_found",
            Self::ConversationNotFound => "conversation_not_found",
            Self::ContextTokenMissing => "context_token_missing",
            Self::MediaNotFound => "media_not_found",
            Self::InvalidCursor => "invalid_cursor",
            Self::InvalidAction => "invalid_action",
            Self::InvalidMediaUpload(_) => "invalid_media_upload",
            Self::WechatApi(_) => "wechat_api_error",
            Self::Normalization(_) => "normalization_failed",
            Self::StateStorage(_) => "state_storage_failed",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::AccountNotFound | Self::LoginNotFound | Self::ConversationNotFound => {
                StatusCode::NOT_FOUND
            }
            Self::ContextTokenMissing
            | Self::MediaNotFound
            | Self::InvalidCursor
            | Self::InvalidAction
            | Self::InvalidMediaUpload(_)
            | Self::WechatApi(_)
            | Self::Normalization(_)
            | Self::StateStorage(_) => StatusCode::BAD_REQUEST,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = Json(ErrorEnvelope {
            error: ErrorBody {
                code: self.code(),
                message: self.to_string(),
            },
        });
        (status, body).into_response()
    }
}
