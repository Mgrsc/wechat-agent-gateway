use std::io::Write;
use std::time::Duration;

use qrcode::QrCode;
use qrcode::render::unicode;
use reqwest::Client;
use thiserror::Error;
use tokio::time::sleep;
use tracing::{debug, info};

use crate::config::RuntimeConfig;
use crate::logging::redact_optional_id;
use crate::models::{LoginStartRequest, LoginStartResponse, LoginStatusResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCommandOptions {
    pub gateway_url: String,
    pub label: String,
    pub wechat_base_url: Option<String>,
    pub poll_interval: Duration,
    pub max_polls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCommandOutcome {
    pub login_id: String,
    pub qrcode_url: String,
    pub status: String,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Serve,
    Login(LoginCommandOptions),
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    #[error("missing value for {0}")]
    MissingValue(&'static str),
    #[error("invalid value for {0}: {1}")]
    InvalidValue(&'static str, String),
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to render qrcode: {0}")]
    Qrcode(String),
    #[error("config failed: {0}")]
    Config(String),
    #[error("login expired before confirmation")]
    LoginExpired,
    #[error("login polling exceeded max polls for {0}")]
    PollExhausted(String),
}

impl Command {
    pub fn parse<I>(args: I) -> Result<Self, CliError>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = args.into_iter();
        let Some(command) = args.next() else {
            return Ok(Self::Serve);
        };

        match command.as_str() {
            "serve" => Ok(Self::Serve),
            "login" => {
                let runtime = RuntimeConfig::from_env()
                    .map_err(|error| CliError::Config(error.to_string()))?;
                let mut options = LoginCommandOptions {
                    gateway_url: runtime.gateway_url,
                    label: "default".to_string(),
                    wechat_base_url: Some(runtime.wechat_base_url),
                    poll_interval: Duration::from_secs(1),
                    max_polls: 120,
                };

                let mut pending = args.peekable();
                while let Some(flag) = pending.next() {
                    match flag.as_str() {
                        "--gateway-url" => {
                            options.gateway_url = pending
                                .next()
                                .ok_or(CliError::MissingValue("--gateway-url"))?;
                        }
                        "--label" => {
                            options.label =
                                pending.next().ok_or(CliError::MissingValue("--label"))?;
                        }
                        "--wechat-base-url" => {
                            options.wechat_base_url = Some(
                                pending
                                    .next()
                                    .ok_or(CliError::MissingValue("--wechat-base-url"))?,
                            );
                        }
                        "--poll-ms" => {
                            let value =
                                pending.next().ok_or(CliError::MissingValue("--poll-ms"))?;
                            let millis = value
                                .parse::<u64>()
                                .map_err(|_| CliError::InvalidValue("--poll-ms", value.clone()))?;
                            options.poll_interval = Duration::from_millis(millis);
                        }
                        "--max-polls" => {
                            let value = pending
                                .next()
                                .ok_or(CliError::MissingValue("--max-polls"))?;
                            options.max_polls = value.parse::<usize>().map_err(|_| {
                                CliError::InvalidValue("--max-polls", value.clone())
                            })?;
                        }
                        _ => return Err(CliError::UnknownCommand(flag)),
                    }
                }

                Ok(Self::Login(options))
            }
            _ => Err(CliError::UnknownCommand(command)),
        }
    }
}

pub async fn run_login_command<W: Write>(
    options: LoginCommandOptions,
    writer: &mut W,
) -> Result<LoginCommandOutcome, CliError> {
    let gateway_url = options.gateway_url.trim_end_matches('/').to_string();
    let client = Client::new();

    info!(gateway_url = %gateway_url, label = %options.label, "starting login command");

    let start = client
        .post(format!("{gateway_url}/v1/accounts/login/start"))
        .json(&LoginStartRequest {
            label: options.label,
            base_url: options.wechat_base_url,
        })
        .send()
        .await?
        .error_for_status()?
        .json::<LoginStartResponse>()
        .await?;

    writeln!(writer, "login_id: {}", start.login_id)?;
    writeln!(writer, "qrcode_url: {}", start.qrcode_url)?;
    writeln!(writer)?;
    writeln!(writer, "{}", render_qrcode(&start.qrcode_url)?)?;

    for poll_index in 0..options.max_polls {
        let status = client
            .get(format!(
                "{gateway_url}/v1/accounts/login/{}",
                start.login_id
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<LoginStatusResponse>()
            .await?;

        debug!(
            login_id = %status.login_id,
            status = %status.status,
            account_id = ?redact_optional_id(status.account_id.as_deref()),
            poll_index,
            "login status polled"
        );

        match status.status.as_str() {
            "confirmed" => {
                if let Some(account_id) = status.account_id.as_deref() {
                    writeln!(writer, "account_id: {account_id}")?;
                }
                return Ok(LoginCommandOutcome {
                    login_id: status.login_id,
                    qrcode_url: start.qrcode_url,
                    status: status.status,
                    account_id: status.account_id,
                });
            }
            "expired" => return Err(CliError::LoginExpired),
            _ => sleep(options.poll_interval).await,
        }
    }

    Err(CliError::PollExhausted(start.login_id))
}

fn render_qrcode(content: &str) -> Result<String, CliError> {
    let code =
        QrCode::new(content.as_bytes()).map_err(|error| CliError::Qrcode(error.to_string()))?;
    Ok(code.render::<unicode::Dense1x2>().quiet_zone(false).build())
}
