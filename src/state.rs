use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::errors::AppError;
use crate::logging::redact_id;
use crate::media::StoredMediaAsset;
use crate::models::{
    Account, ActionBatch, ActionBatchResponse, ActionResult, Conversation, InboundEvent,
    MediaDescriptor, OutboundAction,
};
use crate::wechat_client::LoginStatusPayload;

#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<RwLock<InnerState>>,
    state_path: Arc<Option<PathBuf>>,
}

#[derive(Debug, Default)]
struct InnerState {
    accounts: BTreeMap<String, Account>,
    account_tokens: BTreeMap<String, String>,
    account_cursors: BTreeMap<String, String>,
    conversations: BTreeMap<String, Conversation>,
    media: BTreeMap<String, MediaDescriptor>,
    media_accounts: BTreeMap<String, String>,
    media_assets: BTreeMap<String, StoredMediaAsset>,
    login_sessions: BTreeMap<String, LoginSession>,
    events: Vec<QueuedEvent>,
    last_cursor: u64,
    last_login_id: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistentState {
    accounts: BTreeMap<String, Account>,
    account_tokens: BTreeMap<String, String>,
    account_cursors: BTreeMap<String, String>,
    conversations: BTreeMap<String, Conversation>,
}

#[derive(Debug, Clone)]
struct QueuedEvent {
    cursor: u64,
    event: InboundEvent,
}

#[derive(Debug, Clone)]
pub struct LoginSession {
    pub login_id: String,
    pub label: String,
    pub base_url: String,
    pub qrcode: String,
    pub status: String,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AccountSession {
    pub account_id: String,
    pub base_url: String,
    pub bot_token: String,
    pub wechat_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DispatchContext {
    pub account_id: String,
    pub base_url: String,
    pub bot_token: String,
    pub peer_id: String,
}

impl AppState {
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(RwLock::new(InnerState::default())),
            state_path: Arc::new(None),
        }
    }

    pub fn from_disk(path: impl Into<PathBuf>) -> Result<Self, AppError> {
        let path = path.into();
        let inner = if path.exists() {
            load_persistent_state(&path)?
        } else {
            InnerState::default()
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            state_path: Arc::new(Some(path)),
        })
    }

    pub fn insert_account(&self, account: Account) -> Result<(), AppError> {
        let snapshot = {
            let mut inner = self.inner.write().expect("state lock");
            info!(account_id = %redact_id(&account.account_id), label = %account.label, "account inserted");
            inner.accounts.insert(account.account_id.clone(), account);
            self.snapshot_for_persistence(&inner)
        };
        self.persist_snapshot(snapshot)
    }

    pub fn insert_authenticated_account(
        &self,
        account: Account,
        bot_token: String,
    ) -> Result<(), AppError> {
        let snapshot = {
            let mut inner = self.inner.write().expect("state lock");
            info!(account_id = %redact_id(&account.account_id), label = %account.label, "authenticated account inserted");
            inner
                .account_tokens
                .insert(account.account_id.clone(), bot_token);
            inner.account_cursors.remove(&account.account_id);
            inner.accounts.insert(account.account_id.clone(), account);
            self.snapshot_for_persistence(&inner)
        };
        self.persist_snapshot(snapshot)
    }

    pub fn account_count(&self) -> usize {
        self.inner.read().expect("state lock").accounts.len()
    }

    pub fn list_accounts(&self) -> Vec<Account> {
        self.inner
            .read()
            .expect("state lock")
            .accounts
            .values()
            .cloned()
            .collect()
    }

    pub fn insert_conversation_context_token(
        &self,
        conversation_id: &str,
        context_token: &str,
    ) -> Result<(), AppError> {
        let snapshot = {
            let mut inner = self.inner.write().expect("state lock");
            let conversation = inner
                .conversations
                .entry(conversation_id.to_owned())
                .or_insert_with(|| Conversation {
                    conversation_id: conversation_id.to_owned(),
                    channel: "wechat".into(),
                    account_id: String::new(),
                    peer_id: String::new(),
                    chat_type: "direct".into(),
                    last_context_token: None,
                    last_inbound_at: None,
                    last_outbound_at: None,
                });
            conversation.last_context_token = Some(context_token.to_owned());
            debug!(
                conversation_id = %redact_id(conversation_id),
                "conversation context token updated"
            );
            self.snapshot_for_persistence(&inner)
        };
        self.persist_snapshot(snapshot)
    }

    pub fn enqueue_event(&self, event: InboundEvent) -> Result<(), AppError> {
        let snapshot = {
            let mut inner = self.inner.write().expect("state lock");
            inner.last_cursor += 1;
            let cursor = inner.last_cursor;

            let conversation = inner
                .conversations
                .entry(event.conversation_id.clone())
                .or_insert_with(|| Conversation {
                    conversation_id: event.conversation_id.clone(),
                    channel: event.channel.clone(),
                    account_id: event.account_id.clone(),
                    peer_id: event.peer_id.clone(),
                    chat_type: "direct".into(),
                    last_context_token: None,
                    last_inbound_at: None,
                    last_outbound_at: None,
                });

            conversation.last_context_token = event.context_token.clone();
            conversation.last_inbound_at = Some(event.timestamp_ms);

            debug!(
                cursor,
                event_id = %event.event_id,
                conversation_id = %redact_id(&event.conversation_id),
                "event enqueued"
            );

            for media in &event.media {
                inner
                    .media_accounts
                    .insert(media.media_id.clone(), event.account_id.clone());
                inner.media.insert(media.media_id.clone(), media.clone());
            }
            inner.events.push(QueuedEvent { cursor, event });
            self.snapshot_for_persistence(&inner)
        };
        self.persist_snapshot(snapshot)
    }

    pub fn pull_events(
        &self,
        account_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<InboundEvent>, Option<String>), AppError> {
        let from_cursor = cursor
            .map(|value| value.parse::<u64>().map_err(|_| AppError::InvalidCursor))
            .transpose()?
            .unwrap_or(0);

        let inner = self.inner.read().expect("state lock");
        if !inner.accounts.contains_key(account_id) {
            return Err(AppError::AccountNotFound);
        }

        let mut next_cursor = None;
        let events = inner
            .events
            .iter()
            .filter(|queued| queued.cursor > from_cursor && queued.event.account_id == account_id)
            .take(limit)
            .map(|queued| {
                next_cursor = Some(queued.cursor.to_string());
                queued.event.clone()
            })
            .collect::<Vec<_>>();

        info!(
            account_id = %redact_id(account_id),
            from_cursor,
            limit,
            returned = events.len(),
            next_cursor = ?next_cursor,
            "events pulled"
        );

        Ok((events, next_cursor))
    }

    pub fn insert_media(&self, media: MediaDescriptor) {
        let mut inner = self.inner.write().expect("state lock");
        info!(media_id = %media.media_id, kind = ?media.kind, "media inserted");
        inner.media.insert(media.media_id.clone(), media);
    }

    pub fn insert_media_asset(&self, asset: StoredMediaAsset) -> MediaDescriptor {
        let mut inner = self.inner.write().expect("state lock");
        let descriptor = asset.descriptor.clone();
        let account_id = asset.account_id.clone();
        info!(
            media_id = %descriptor.media_id,
            kind = ?descriptor.kind,
            account_id = %redact_id(&asset.account_id),
            size = ?descriptor.size,
            "media asset inserted"
        );
        inner
            .media_assets
            .insert(descriptor.media_id.clone(), asset);
        inner
            .media_accounts
            .insert(descriptor.media_id.clone(), account_id.clone());
        inner
            .media
            .insert(descriptor.media_id.clone(), descriptor.clone());
        descriptor
    }

    pub fn has_media_asset(&self, media_id: &str) -> bool {
        self.inner
            .read()
            .expect("state lock")
            .media_assets
            .contains_key(media_id)
    }

    pub fn get_media_asset(&self, media_id: &str) -> Option<StoredMediaAsset> {
        self.inner
            .read()
            .expect("state lock")
            .media_assets
            .get(media_id)
            .cloned()
    }

    pub fn get_media(&self, media_id: &str) -> Option<MediaDescriptor> {
        self.inner
            .read()
            .expect("state lock")
            .media
            .get(media_id)
            .cloned()
    }

    pub fn get_media_account_id(&self, media_id: &str) -> Option<String> {
        self.inner
            .read()
            .expect("state lock")
            .media_accounts
            .get(media_id)
            .cloned()
    }

    pub fn create_login_session(
        &self,
        label: String,
        base_url: String,
        qrcode: String,
    ) -> LoginSession {
        let mut inner = self.inner.write().expect("state lock");
        inner.last_login_id += 1;
        let login_id = format!("login_{}", inner.last_login_id);
        let session = LoginSession {
            login_id: login_id.clone(),
            label,
            base_url,
            qrcode,
            status: "waiting".into(),
            account_id: None,
        };

        info!(login_id = %session.login_id, qrcode = "<redacted>", "login session created");
        inner.login_sessions.insert(login_id, session.clone());
        session
    }

    pub fn get_login_session(&self, login_id: &str) -> Result<LoginSession, AppError> {
        self.inner
            .read()
            .expect("state lock")
            .login_sessions
            .get(login_id)
            .cloned()
            .ok_or(AppError::LoginNotFound)
    }

    pub fn get_account_session(&self, account_id: &str) -> Result<AccountSession, AppError> {
        let inner = self.inner.read().expect("state lock");
        let account = inner
            .accounts
            .get(account_id)
            .cloned()
            .ok_or(AppError::AccountNotFound)?;
        let bot_token = inner
            .account_tokens
            .get(account_id)
            .cloned()
            .ok_or(AppError::AccountNotFound)?;

        Ok(AccountSession {
            account_id: account.account_id,
            base_url: account.base_url,
            bot_token,
            wechat_cursor: inner.account_cursors.get(account_id).cloned(),
        })
    }

    pub fn update_account_cursor(&self, account_id: &str, cursor: String) -> Result<(), AppError> {
        let snapshot = {
            let mut inner = self.inner.write().expect("state lock");
            if !inner.accounts.contains_key(account_id) {
                return Err(AppError::AccountNotFound);
            }
            debug!(account_id = %redact_id(account_id), cursor = "<redacted>", "wechat cursor updated");
            inner.account_cursors.insert(account_id.to_owned(), cursor);
            self.snapshot_for_persistence(&inner)
        };
        self.persist_snapshot(snapshot)
    }

    pub fn get_dispatch_context(
        &self,
        conversation_id: &str,
    ) -> Result<Option<DispatchContext>, AppError> {
        let inner = self.inner.read().expect("state lock");
        let conversation = match inner.conversations.get(conversation_id) {
            Some(value) => value,
            None => return Ok(None),
        };

        if conversation.account_id.is_empty() || conversation.peer_id.is_empty() {
            return Ok(None);
        }

        let account = match inner.accounts.get(&conversation.account_id) {
            Some(value) => value,
            None => return Ok(None),
        };
        let bot_token = match inner.account_tokens.get(&conversation.account_id) {
            Some(value) => value.clone(),
            None => return Ok(None),
        };

        Ok(Some(DispatchContext {
            account_id: conversation.account_id.clone(),
            base_url: account.base_url.clone(),
            bot_token,
            peer_id: conversation.peer_id.clone(),
        }))
    }

    pub fn get_conversation(&self, conversation_id: &str) -> Result<Conversation, AppError> {
        self.inner
            .read()
            .expect("state lock")
            .conversations
            .get(conversation_id)
            .cloned()
            .ok_or(AppError::ConversationNotFound)
    }

    pub fn mark_outbound(
        &self,
        conversation_id: &str,
        context_token: Option<&str>,
    ) -> Result<(), AppError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis() as u64;

        let snapshot = {
            let mut inner = self.inner.write().expect("state lock");
            let conversation = inner
                .conversations
                .get_mut(conversation_id)
                .ok_or(AppError::InvalidAction)?;
            conversation.last_outbound_at = Some(now_ms);
            if let Some(context_token) = context_token {
                conversation.last_context_token = Some(context_token.to_owned());
            }
            debug!(
                conversation_id = %redact_id(conversation_id),
                now_ms, "conversation outbound marker updated"
            );
            self.snapshot_for_persistence(&inner)
        };
        self.persist_snapshot(snapshot)
    }

    pub fn update_login_session_status(
        &self,
        login_id: &str,
        payload: &LoginStatusPayload,
    ) -> Result<LoginSession, AppError> {
        let mut inner = self.inner.write().expect("state lock");
        let (label, fallback_base_url, session_snapshot) = {
            let session = inner
                .login_sessions
                .get_mut(login_id)
                .ok_or(AppError::LoginNotFound)?;

            session.status = payload.status.clone();
            session.account_id = payload.ilink_bot_id.clone();

            (
                session.label.clone(),
                session.base_url.clone(),
                session.clone(),
            )
        };

        if payload.status == "confirmed" {
            let account_id = payload
                .ilink_bot_id
                .clone()
                .ok_or_else(|| AppError::WechatApi("confirmed login missing bot id".into()))?;
            let bot_token = payload
                .bot_token
                .clone()
                .ok_or_else(|| AppError::WechatApi("confirmed login missing bot token".into()))?;
            let account = Account {
                account_id: account_id.clone(),
                label,
                enabled: true,
                configured: true,
                base_url: payload.baseurl.clone().unwrap_or(fallback_base_url),
            };

            inner.account_tokens.insert(account_id.clone(), bot_token);
            inner.accounts.insert(account_id.clone(), account);
            info!(
                login_id,
                account_id = %redact_id(&account_id), "login session confirmed and account persisted"
            );
        } else {
            info!(login_id, status = %payload.status, "login session status updated");
        }

        let snapshot = self.snapshot_for_persistence(&inner);
        self.persist_snapshot(snapshot)?;

        Ok(session_snapshot)
    }

    pub fn apply_actions(&self, batch: &ActionBatch) -> Result<ActionBatchResponse, AppError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis() as u64;

        let mut results = Vec::with_capacity(batch.actions.len());
        let mut inner = self.inner.write().expect("state lock");
        let conversation = inner
            .conversations
            .entry(batch.conversation_id.clone())
            .or_insert_with(|| Conversation {
                conversation_id: batch.conversation_id.clone(),
                channel: "wechat".into(),
                account_id: String::new(),
                peer_id: String::new(),
                chat_type: "direct".into(),
                last_context_token: None,
                last_inbound_at: None,
                last_outbound_at: None,
            });

        conversation.last_context_token = batch.context_token.clone();
        conversation.last_outbound_at = Some(now_ms);

        for (index, action) in batch.actions.iter().enumerate() {
            match action {
                OutboundAction::Typing { .. } => {
                    results.push(ActionResult {
                        action_type: "typing",
                        ok: true,
                        message_id: None,
                    });
                }
                OutboundAction::SendText { .. } => {
                    results.push(ActionResult {
                        action_type: "send_text",
                        ok: true,
                        message_id: Some(format!("msg_{index}")),
                    });
                }
                OutboundAction::SendMedia { media_id, .. } => {
                    if !inner.media_assets.contains_key(media_id) {
                        return Err(AppError::MediaNotFound);
                    }
                    results.push(ActionResult {
                        action_type: "send_media",
                        ok: true,
                        message_id: Some(format!("msg_{index}")),
                    });
                }
            }
        }

        info!(
            conversation_id = %batch.conversation_id,
            action_count = batch.actions.len(),
            "action batch applied"
        );

        let snapshot = self.snapshot_for_persistence(&inner);
        self.persist_snapshot(snapshot)?;

        Ok(ActionBatchResponse { ok: true, results })
    }

    fn snapshot_for_persistence(&self, inner: &InnerState) -> PersistentState {
        PersistentState {
            accounts: inner.accounts.clone(),
            account_tokens: inner.account_tokens.clone(),
            account_cursors: inner.account_cursors.clone(),
            conversations: inner.conversations.clone(),
        }
    }

    fn persist_snapshot(&self, snapshot: PersistentState) -> Result<(), AppError> {
        let Some(path) = self.state_path.as_ref().as_ref() else {
            return Ok(());
        };
        persist_snapshot(path, &snapshot)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::in_memory()
    }
}

fn load_persistent_state(path: &Path) -> Result<InnerState, AppError> {
    let bytes = fs::read(path).map_err(|error| AppError::StateStorage(error.to_string()))?;
    let snapshot = serde_json::from_slice::<PersistentState>(&bytes)
        .map_err(|error| AppError::StateStorage(error.to_string()))?;
    Ok(InnerState {
        accounts: snapshot.accounts,
        account_tokens: snapshot.account_tokens,
        account_cursors: snapshot.account_cursors,
        conversations: snapshot.conversations,
        media: BTreeMap::new(),
        media_accounts: BTreeMap::new(),
        media_assets: BTreeMap::new(),
        login_sessions: BTreeMap::new(),
        events: Vec::new(),
        last_cursor: 0,
        last_login_id: 0,
    })
}

fn persist_snapshot(path: &Path, snapshot: &PersistentState) -> Result<(), AppError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| AppError::StateStorage(error.to_string()))?;
    }

    let bytes = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| AppError::StateStorage(error.to_string()))?;
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, bytes).map_err(|error| AppError::StateStorage(error.to_string()))?;
    fs::rename(&tmp_path, path).map_err(|error| AppError::StateStorage(error.to_string()))?;
    Ok(())
}
