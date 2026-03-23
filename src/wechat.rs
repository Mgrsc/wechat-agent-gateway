use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::models::{EventKind, InboundEvent, MediaDescriptor, MediaRawRef};

#[derive(Debug, Error)]
pub enum NormalizeError {
    #[error("message has no supported items")]
    EmptyMessage,
    #[error("message item type {0} is unsupported")]
    UnsupportedItem(u8),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WechatMessage {
    pub seq: u64,
    pub message_id: u64,
    pub from_user_id: String,
    pub to_user_id: String,
    pub create_time_ms: u64,
    pub item_list: Vec<WechatItem>,
    pub context_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WechatUpdatesResponse {
    pub msgs: Vec<WechatMessage>,
    pub get_updates_buf: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WechatItem {
    #[serde(rename = "type")]
    pub item_type: u8,
    pub text_item: Option<TextItem>,
    pub image_item: Option<ImageItem>,
    pub voice_item: Option<VoiceItem>,
    pub file_item: Option<FileItem>,
    pub video_item: Option<VideoItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextItem {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageItem {
    pub media: Option<WechatMediaRef>,
    pub mid_size: Option<u64>,
    pub thumb_size: Option<u64>,
    pub thumb_height: Option<u32>,
    pub thumb_width: Option<u32>,
    pub hd_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceItem {
    pub media: Option<WechatMediaRef>,
    pub playtime: Option<u64>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileItem {
    pub media: Option<WechatMediaRef>,
    pub file_name: Option<String>,
    pub md5: Option<String>,
    pub len: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoItem {
    pub media: Option<WechatMediaRef>,
    pub video_size: Option<u64>,
    pub play_length: Option<u64>,
    pub video_md5: Option<String>,
    pub thumb_media: Option<WechatMediaRef>,
    pub thumb_size: Option<u64>,
    pub thumb_height: Option<u32>,
    pub thumb_width: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WechatMediaRef {
    pub encrypt_query_param: Option<String>,
    pub aes_key: Option<String>,
}

pub fn normalize_message(message: &WechatMessage) -> Result<InboundEvent, NormalizeError> {
    let conversation_id = format!("wechat:{}:{}", message.to_user_id, message.from_user_id);
    let first_item = message
        .item_list
        .first()
        .ok_or(NormalizeError::EmptyMessage)?;

    let (kind, text, media) = normalize_items(message.message_id, &message.item_list)?;

    Ok(InboundEvent {
        event_id: format!("evt_{}", message.message_id),
        conversation_id,
        channel: "wechat".into(),
        account_id: message.to_user_id.clone(),
        peer_id: message.from_user_id.clone(),
        message_id: message.message_id.to_string(),
        context_token: message.context_token.clone(),
        kind,
        text,
        quoted_text: None,
        media,
        timestamp_ms: message.create_time_ms,
        raw_ref: json!({
            "platform": "wechat",
            "seq": message.seq,
            "item_type": first_item.item_type
        }),
    })
}

fn normalize_items(
    message_id: u64,
    items: &[WechatItem],
) -> Result<(EventKind, Option<String>, Vec<MediaDescriptor>), NormalizeError> {
    let mut kind = None;
    let mut text = None;
    let mut media = Vec::new();

    for (index, item) in items.iter().enumerate() {
        match item.item_type {
            1 => {
                let body = item
                    .text_item
                    .as_ref()
                    .map(|value| value.text.clone())
                    .unwrap_or_default();
                kind.get_or_insert(EventKind::Text);
                if !body.is_empty() {
                    text = Some(body);
                }
            }
            2 => {
                let image = item
                    .image_item
                    .as_ref()
                    .ok_or(NormalizeError::UnsupportedItem(2))?;
                kind.get_or_insert(EventKind::Image);
                media.push(MediaDescriptor {
                    media_id: format!("med_{message_id}_{index}"),
                    kind: EventKind::Image,
                    filename: None,
                    mime: None,
                    size: image.hd_size.or(image.mid_size).or(image.thumb_size),
                    width: image.thumb_width,
                    height: image.thumb_height,
                    duration_ms: None,
                    checksum_md5: None,
                    transcript: None,
                    raw_ref: media_ref(image.media.as_ref(), None),
                });
            }
            3 => {
                let voice = item
                    .voice_item
                    .as_ref()
                    .ok_or(NormalizeError::UnsupportedItem(3))?;
                kind.get_or_insert(EventKind::Voice);
                if voice.text.is_some() {
                    text = voice.text.clone();
                }
                media.push(MediaDescriptor {
                    media_id: format!("med_{message_id}_{index}"),
                    kind: EventKind::Voice,
                    filename: None,
                    mime: None,
                    size: None,
                    width: None,
                    height: None,
                    duration_ms: voice.playtime,
                    checksum_md5: None,
                    transcript: voice.text.clone(),
                    raw_ref: media_ref(voice.media.as_ref(), None),
                });
            }
            4 => {
                let file = item
                    .file_item
                    .as_ref()
                    .ok_or(NormalizeError::UnsupportedItem(4))?;
                kind.get_or_insert(EventKind::File);
                media.push(MediaDescriptor {
                    media_id: format!("med_{message_id}_{index}"),
                    kind: EventKind::File,
                    filename: file.file_name.clone(),
                    mime: None,
                    size: file
                        .len
                        .as_deref()
                        .and_then(|value| value.parse::<u64>().ok()),
                    width: None,
                    height: None,
                    duration_ms: None,
                    checksum_md5: file.md5.clone(),
                    transcript: None,
                    raw_ref: media_ref(file.media.as_ref(), None),
                });
            }
            5 => {
                let video = item
                    .video_item
                    .as_ref()
                    .ok_or(NormalizeError::UnsupportedItem(5))?;
                kind.get_or_insert(EventKind::Video);
                media.push(MediaDescriptor {
                    media_id: format!("med_{message_id}_{index}"),
                    kind: EventKind::Video,
                    filename: None,
                    mime: None,
                    size: video.video_size,
                    width: video.thumb_width,
                    height: video.thumb_height,
                    duration_ms: video.play_length.map(|value| value * 1000),
                    checksum_md5: video.video_md5.clone(),
                    transcript: None,
                    raw_ref: media_ref(video.media.as_ref(), video.thumb_media.as_ref()),
                });
            }
            other => return Err(NormalizeError::UnsupportedItem(other)),
        }
    }

    let kind = kind.ok_or(NormalizeError::EmptyMessage)?;
    Ok((kind, text, media))
}

fn media_ref(media: Option<&WechatMediaRef>, thumb_media: Option<&WechatMediaRef>) -> MediaRawRef {
    MediaRawRef {
        encrypt_query_param: media.and_then(|value| value.encrypt_query_param.clone()),
        aes_key: media.and_then(|value| value.aes_key.clone()),
        thumb_encrypt_query_param: thumb_media.and_then(|value| value.encrypt_query_param.clone()),
        thumb_aes_key: thumb_media.and_then(|value| value.aes_key.clone()),
    }
}
