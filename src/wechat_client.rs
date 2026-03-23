use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

use crate::errors::AppError;
use crate::logging::{redact_id, redact_optional_id, redact_text, sanitize_url};
use crate::media::{
    UploadedMediaRef, aes_ecb_padded_size, base64_encode, decrypt_aes_ecb_pkcs7,
    encrypt_aes_ecb_pkcs7, generate_aes_key, hex_encode, parse_aes_key_base64, random_hex_16,
};
use crate::models::{EventKind, MediaDescriptor, TypingStatus};
use crate::wechat::WechatUpdatesResponse;

pub const DEFAULT_WECHAT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
pub const DEFAULT_WECHAT_CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com";
const CHANNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct WechatClient {
    client: reqwest::Client,
    base_url: String,
    cdn_base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginStartPayload {
    pub qrcode: String,
    #[serde(rename = "qrcode_img_content")]
    pub qrcode_url: String,
    pub ret: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginStatusPayload {
    pub baseurl: Option<String>,
    pub bot_token: Option<String>,
    pub ilink_bot_id: Option<String>,
    pub ilink_user_id: Option<String>,
    pub ret: i32,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetConfigPayload {
    pub ret: i32,
    pub typing_ticket: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetUploadUrlPayload {
    pub upload_param: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UploadMediaType {
    Image = 1,
    Video = 2,
    File = 3,
}

impl WechatClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_cdn(base_url, None::<String>)
    }

    pub fn with_cdn(base_url: impl Into<String>, cdn_base_url: Option<impl Into<String>>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            cdn_base_url: cdn_base_url
                .map(|value| value.into().trim_end_matches('/').to_owned())
                .unwrap_or_default(),
        }
        .with_default_cdn()
    }

    pub async fn start_login(&self) -> Result<LoginStartPayload, AppError> {
        let url = format!("{}/ilink/bot/get_bot_qrcode?bot_type=3", self.base_url);
        info!(url = %sanitize_url(&url), "requesting login qrcode");

        let payload = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .json::<LoginStartPayload>()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        if payload.ret != 0 {
            return Err(AppError::WechatApi(format!(
                "get_bot_qrcode returned ret={}",
                payload.ret
            )));
        }

        debug!(
            qrcode = "<redacted>",
            qrcode_url = %sanitize_url(&payload.qrcode_url),
            "received login qrcode"
        );
        Ok(payload)
    }

    pub async fn get_login_status(&self, qrcode: &str) -> Result<LoginStatusPayload, AppError> {
        let url = format!(
            "{}/ilink/bot/get_qrcode_status?qrcode={qrcode}",
            self.base_url
        );
        let mut headers = HeaderMap::new();
        headers.insert("iLink-App-ClientVersion", HeaderValue::from_static("1"));

        info!(url = %sanitize_url(&url), qrcode = "<redacted>", "polling login status");

        let payload = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .json::<LoginStatusPayload>()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        if payload.ret != 0 {
            return Err(AppError::WechatApi(format!(
                "get_qrcode_status returned ret={}",
                payload.ret
            )));
        }

        debug!(
            status = %payload.status,
            account_id = ?redact_optional_id(payload.ilink_bot_id.as_deref()),
            user_id = ?redact_optional_id(payload.ilink_user_id.as_deref()),
            "received login status"
        );

        Ok(payload)
    }

    pub async fn get_updates(
        &self,
        bot_token: &str,
        get_updates_buf: Option<&str>,
    ) -> Result<WechatUpdatesResponse, AppError> {
        let url = format!("{}/ilink/bot/getupdates", self.base_url);
        let mut headers = HeaderMap::new();
        headers.insert(
            "AuthorizationType",
            HeaderValue::from_static("ilink_bot_token"),
        );
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {bot_token}"))
                .map_err(|error| AppError::WechatApi(error.to_string()))?,
        );
        headers.insert("X-WECHAT-UIN", random_wechat_uin()?);

        let mut body = serde_json::json!({
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        if let Some(cursor) = get_updates_buf {
            body["get_updates_buf"] = serde_json::Value::String(cursor.to_owned());
        }

        info!(
            url = %sanitize_url(&url),
            has_cursor = get_updates_buf.is_some(),
            "polling wechat updates"
        );

        let payload = self
            .client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .json::<WechatUpdatesResponse>()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        debug!(
            received = payload.msgs.len(),
            next_wechat_cursor = ?payload.get_updates_buf,
            "received wechat updates"
        );

        Ok(payload)
    }

    pub async fn get_config(
        &self,
        bot_token: &str,
        ilink_user_id: &str,
        context_token: &str,
    ) -> Result<GetConfigPayload, AppError> {
        let url = format!("{}/ilink/bot/getconfig", self.base_url);
        let headers = auth_headers(bot_token)?;
        let body = serde_json::json!({
            "ilink_user_id": ilink_user_id,
            "context_token": context_token,
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        info!(
            url = %sanitize_url(&url),
            ilink_user_id = %redact_id(ilink_user_id),
            "requesting typing config"
        );

        let payload = self
            .client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .json::<GetConfigPayload>()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        if payload.ret != 0 {
            return Err(AppError::WechatApi(format!(
                "getconfig returned ret={}",
                payload.ret
            )));
        }

        Ok(payload)
    }

    pub async fn send_typing(
        &self,
        bot_token: &str,
        ilink_user_id: &str,
        typing_ticket: &str,
        status: TypingStatus,
    ) -> Result<(), AppError> {
        let url = format!("{}/ilink/bot/sendtyping", self.base_url);
        let headers = auth_headers(bot_token)?;
        let body = serde_json::json!({
            "ilink_user_id": ilink_user_id,
            "typing_ticket": typing_ticket,
            "status": match status {
                TypingStatus::Start => 1,
                TypingStatus::Stop => 2,
            },
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        info!(
            url = %sanitize_url(&url),
            ilink_user_id = %redact_id(ilink_user_id),
            status = ?status,
            "sending typing state"
        );

        self.client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        Ok(())
    }

    pub async fn send_text(
        &self,
        bot_token: &str,
        to_user_id: &str,
        context_token: &str,
        text: &str,
    ) -> Result<String, AppError> {
        let url = format!("{}/ilink/bot/sendmessage", self.base_url);
        let headers = auth_headers(bot_token)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis();
        let client_id = format!("wechat-agent-gateway:{now_ms}");
        let body = serde_json::json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": client_id,
                "message_type": 2,
                "message_state": 2,
                "item_list": [
                    {
                        "type": 1,
                        "text_item": {
                            "text": text
                        }
                    }
                ],
                "context_token": context_token
            },
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        info!(
            url = %sanitize_url(&url),
            to_user_id = %redact_id(to_user_id),
            text = %redact_text(text),
            "sending text message"
        );

        self.client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        Ok(client_id)
    }

    async fn upload_media(
        &self,
        bot_token: &str,
        to_user_id: &str,
        plaintext: &[u8],
        media_type: UploadMediaType,
    ) -> Result<UploadedMediaRef, AppError> {
        let url = format!("{}/ilink/bot/getuploadurl", self.base_url);
        let headers = auth_headers(bot_token)?;
        let filekey = random_hex_16();
        let aes_key = generate_aes_key();
        let aes_key_hex = hex_encode(&aes_key);
        let raw_md5 = format!("{:x}", md5::compute(plaintext));
        let ciphertext_size = aes_ecb_padded_size(plaintext.len());
        let body = serde_json::json!({
            "filekey": filekey,
            "media_type": media_type as u8,
            "to_user_id": to_user_id,
            "rawsize": plaintext.len(),
            "rawfilemd5": raw_md5,
            "filesize": ciphertext_size,
            "no_need_thumb": true,
            "aeskey": aes_key_hex,
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        info!(
            url = %sanitize_url(&url),
            media_kind = ?media_type,
            to_user_id = %redact_id(to_user_id),
            plaintext_size = plaintext.len(),
            "requesting media upload url"
        );

        let upload = self
            .client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .json::<GetUploadUrlPayload>()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        let upload_param = upload
            .upload_param
            .ok_or_else(|| AppError::WechatApi("getuploadurl returned no upload_param".into()))?;
        let ciphertext = encrypt_aes_ecb_pkcs7(plaintext, &aes_key)?;
        let cdn_url = format!(
            "{}/c2c/upload?encrypted_query_param={}&filekey={}",
            self.cdn_base_url, upload_param, filekey
        );

        info!(
            cdn_url = %sanitize_url(&cdn_url),
            media_kind = ?media_type,
            ciphertext_size = ciphertext.len(),
            "uploading media to cdn"
        );

        let response = self
            .client
            .post(cdn_url)
            .header("content-type", "application/octet-stream")
            .body(ciphertext)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        let download_encrypted_query_param = response
            .headers()
            .get("x-encrypted-param")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| AppError::WechatApi("cdn upload missing x-encrypted-param".into()))?;

        Ok(UploadedMediaRef {
            download_encrypted_query_param,
            aes_key_hex: aes_key_hex.clone(),
            aes_key_base64: base64_encode(aes_key_hex.as_bytes()),
            plaintext_size: plaintext.len(),
            ciphertext_size,
        })
    }

    pub async fn upload_image(
        &self,
        bot_token: &str,
        to_user_id: &str,
        plaintext: &[u8],
    ) -> Result<UploadedMediaRef, AppError> {
        self.upload_media(bot_token, to_user_id, plaintext, UploadMediaType::Image)
            .await
    }

    pub async fn upload_video(
        &self,
        bot_token: &str,
        to_user_id: &str,
        plaintext: &[u8],
    ) -> Result<UploadedMediaRef, AppError> {
        self.upload_media(bot_token, to_user_id, plaintext, UploadMediaType::Video)
            .await
    }

    pub async fn upload_file(
        &self,
        bot_token: &str,
        to_user_id: &str,
        plaintext: &[u8],
    ) -> Result<UploadedMediaRef, AppError> {
        self.upload_media(bot_token, to_user_id, plaintext, UploadMediaType::File)
            .await
    }

    pub async fn send_image(
        &self,
        bot_token: &str,
        to_user_id: &str,
        context_token: &str,
        caption: Option<&str>,
        uploaded: &UploadedMediaRef,
    ) -> Result<String, AppError> {
        if let Some(caption) = caption.filter(|text| !text.is_empty()) {
            self.send_text(bot_token, to_user_id, context_token, caption)
                .await?;
        }

        let url = format!("{}/ilink/bot/sendmessage", self.base_url);
        let headers = auth_headers(bot_token)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis();
        let client_id = format!("wechat-agent-gateway:{now_ms}");
        let body = serde_json::json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": client_id,
                "message_type": 2,
                "message_state": 2,
                "item_list": [
                    {
                        "type": 2,
                        "image_item": {
                            "media": {
                                "encrypt_query_param": uploaded.download_encrypted_query_param,
                                "aes_key": uploaded.aes_key_base64,
                                "encrypt_type": 1
                            },
                            "mid_size": uploaded.ciphertext_size
                        }
                    }
                ],
                "context_token": context_token
            },
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        info!(
            url = %sanitize_url(&url),
            to_user_id = %redact_id(to_user_id),
            "sending image message"
        );

        self.client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        Ok(client_id)
    }

    pub async fn send_video(
        &self,
        bot_token: &str,
        to_user_id: &str,
        context_token: &str,
        caption: Option<&str>,
        uploaded: &UploadedMediaRef,
    ) -> Result<String, AppError> {
        if let Some(caption) = caption.filter(|text| !text.is_empty()) {
            self.send_text(bot_token, to_user_id, context_token, caption)
                .await?;
        }

        let url = format!("{}/ilink/bot/sendmessage", self.base_url);
        let headers = auth_headers(bot_token)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis();
        let client_id = format!("wechat-agent-gateway:{now_ms}");
        let body = serde_json::json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": client_id,
                "message_type": 2,
                "message_state": 2,
                "item_list": [
                    {
                        "type": 5,
                        "video_item": {
                            "media": {
                                "encrypt_query_param": uploaded.download_encrypted_query_param,
                                "aes_key": uploaded.aes_key_base64,
                                "encrypt_type": 1
                            },
                            "video_size": uploaded.ciphertext_size
                        }
                    }
                ],
                "context_token": context_token
            },
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        info!(
            url = %sanitize_url(&url),
            to_user_id = %redact_id(to_user_id),
            "sending video message"
        );

        self.client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        Ok(client_id)
    }

    pub async fn send_file(
        &self,
        bot_token: &str,
        to_user_id: &str,
        context_token: &str,
        caption: Option<&str>,
        file_name: &str,
        uploaded: &UploadedMediaRef,
    ) -> Result<String, AppError> {
        if let Some(caption) = caption.filter(|text| !text.is_empty()) {
            self.send_text(bot_token, to_user_id, context_token, caption)
                .await?;
        }

        let url = format!("{}/ilink/bot/sendmessage", self.base_url);
        let headers = auth_headers(bot_token)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis();
        let client_id = format!("wechat-agent-gateway:{now_ms}");
        let body = serde_json::json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": client_id,
                "message_type": 2,
                "message_state": 2,
                "item_list": [
                    {
                        "type": 4,
                        "file_item": {
                            "media": {
                                "encrypt_query_param": uploaded.download_encrypted_query_param,
                                "aes_key": uploaded.aes_key_base64,
                                "encrypt_type": 1
                            },
                            "file_name": file_name,
                            "len": uploaded.plaintext_size.to_string()
                        }
                    }
                ],
                "context_token": context_token
            },
            "base_info": {
                "channel_version": CHANNEL_VERSION
            }
        });

        info!(
            url = %sanitize_url(&url),
            to_user_id = %redact_id(to_user_id),
            file_name = %redact_text(file_name),
            "sending file message"
        );

        self.client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        Ok(client_id)
    }

    pub async fn download_media(&self, descriptor: &MediaDescriptor) -> Result<Vec<u8>, AppError> {
        let encrypted_query_param = descriptor
            .raw_ref
            .encrypt_query_param
            .as_deref()
            .ok_or_else(|| AppError::WechatApi("media missing encrypt_query_param".into()))?;
        let aes_key = descriptor
            .raw_ref
            .aes_key
            .as_deref()
            .ok_or_else(|| AppError::WechatApi("media missing aes_key".into()))?;
        let key = parse_aes_key_base64(aes_key)?;
        let url = format!(
            "{}/c2c/download?encrypted_query_param={}",
            self.cdn_base_url,
            urlencoding::encode(encrypted_query_param)
        );

        info!(
            url = %sanitize_url(&url),
            kind = ?descriptor.kind,
            media_id = %descriptor.media_id,
            "downloading inbound media"
        );

        let ciphertext = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .error_for_status()
            .map_err(|error| AppError::WechatApi(error.to_string()))?
            .bytes()
            .await
            .map_err(|error| AppError::WechatApi(error.to_string()))?;

        match descriptor.kind {
            EventKind::Image | EventKind::Voice | EventKind::File | EventKind::Video => {
                decrypt_aes_ecb_pkcs7(&ciphertext, &key)
            }
            EventKind::Text => Err(AppError::WechatApi(
                "text events have no downloadable media".into(),
            )),
        }
    }
}

impl WechatClient {
    fn with_default_cdn(mut self) -> Self {
        self.cdn_base_url = if self.base_url.starts_with("http://127.0.0.1")
            || self.base_url.starts_with("http://localhost")
        {
            self.base_url.clone()
        } else {
            DEFAULT_WECHAT_CDN_BASE_URL.to_string()
        };
        self
    }
}

fn auth_headers(bot_token: &str) -> Result<HeaderMap, AppError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "AuthorizationType",
        HeaderValue::from_static("ilink_bot_token"),
    );
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {bot_token}"))
            .map_err(|error| AppError::WechatApi(error.to_string()))?,
    );
    headers.insert("X-WECHAT-UIN", random_wechat_uin()?);
    Ok(headers)
}

fn random_wechat_uin() -> Result<HeaderValue, AppError> {
    let value = rand::random::<u32>().to_string();
    HeaderValue::from_str(&base64_encode(value.as_bytes()))
        .map_err(|error| AppError::WechatApi(error.to_string()))
}
