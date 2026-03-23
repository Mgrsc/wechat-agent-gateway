use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Text,
    Image,
    Voice,
    File,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub account_id: String,
    pub label: String,
    pub enabled: bool,
    pub configured: bool,
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conversation {
    pub conversation_id: String,
    pub channel: String,
    pub account_id: String,
    pub peer_id: String,
    pub chat_type: String,
    pub last_context_token: Option<String>,
    pub last_inbound_at: Option<u64>,
    pub last_outbound_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MediaRawRef {
    pub encrypt_query_param: Option<String>,
    pub aes_key: Option<String>,
    pub thumb_encrypt_query_param: Option<String>,
    pub thumb_aes_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaDescriptor {
    pub media_id: String,
    pub kind: EventKind,
    pub filename: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub checksum_md5: Option<String>,
    pub transcript: Option<String>,
    pub raw_ref: MediaRawRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InboundEvent {
    pub event_id: String,
    pub conversation_id: String,
    pub channel: String,
    pub account_id: String,
    pub peer_id: String,
    pub message_id: String,
    pub context_token: Option<String>,
    pub kind: EventKind,
    pub text: Option<String>,
    pub quoted_text: Option<String>,
    pub media: Vec<MediaDescriptor>,
    pub timestamp_ms: u64,
    pub raw_ref: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub account_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountsResponse {
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PullEventsRequest {
    pub cursor: Option<String>,
    pub account_id: String,
    pub wait_ms: u64,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginStartRequest {
    pub label: String,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginStartResponse {
    pub login_id: String,
    pub qrcode_url: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginStatusResponse {
    pub login_id: String,
    pub status: String,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaUploadResponse {
    pub media_id: String,
    pub kind: EventKind,
    pub size: u64,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PullEventsResponse {
    pub events: Vec<InboundEvent>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionBatch {
    pub conversation_id: String,
    pub context_token: Option<String>,
    pub actions: Vec<OutboundAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboundAction {
    Typing {
        status: TypingStatus,
    },
    SendText {
        text: String,
    },
    SendMedia {
        media_id: String,
        caption: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TypingStatus {
    Start,
    Stop,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionResult {
    #[serde(rename = "type")]
    pub action_type: &'static str,
    pub ok: bool,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionBatchResponse {
    pub ok: bool,
    pub results: Vec<ActionResult>,
}
