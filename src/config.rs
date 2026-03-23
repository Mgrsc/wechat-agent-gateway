use std::collections::BTreeMap;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use thiserror::Error;

use crate::wechat_client::{DEFAULT_WECHAT_BASE_URL, DEFAULT_WECHAT_CDN_BASE_URL};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub bind: String,
    pub port: u16,
    pub gateway_url: String,
    pub state_path: PathBuf,
    pub wechat_base_url: String,
    pub wechat_cdn_base_url: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid value for {0}: {1}")]
    InvalidValue(&'static str, String),
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_map(env::vars())
    }

    pub fn from_env_map<I, K, V>(vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let vars = vars
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect::<BTreeMap<String, String>>();
        let bind = vars
            .get("WECHAT_GATEWAY_BIND")
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = vars
            .get("WECHAT_GATEWAY_PORT")
            .map(|value| {
                value
                    .parse::<u16>()
                    .map_err(|_| ConfigError::InvalidValue("WECHAT_GATEWAY_PORT", value.clone()))
            })
            .transpose()?
            .unwrap_or(8080);
        let gateway_url = vars
            .get("WECHAT_GATEWAY_URL")
            .cloned()
            .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
        let state_path = vars
            .get("WECHAT_GATEWAY_STATE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(default_state_path);
        let wechat_base_url = vars
            .get("WECHAT_GATEWAY_BASE_URL")
            .cloned()
            .unwrap_or_else(|| DEFAULT_WECHAT_BASE_URL.to_string());
        let wechat_cdn_base_url = vars
            .get("WECHAT_GATEWAY_CDN_BASE_URL")
            .cloned()
            .unwrap_or_else(|| DEFAULT_WECHAT_CDN_BASE_URL.to_string());

        Ok(Self {
            bind,
            port,
            gateway_url,
            state_path,
            wechat_base_url,
            wechat_cdn_base_url,
        })
    }

    pub fn listen_addr(&self) -> Result<SocketAddr, ConfigError> {
        let ip = self
            .bind
            .parse::<IpAddr>()
            .map_err(|_| ConfigError::InvalidValue("WECHAT_GATEWAY_BIND", self.bind.clone()))?;
        Ok(SocketAddr::from((ip, self.port)))
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 8080,
            gateway_url: "http://127.0.0.1:8080".to_string(),
            state_path: default_state_path(),
            wechat_base_url: DEFAULT_WECHAT_BASE_URL.to_string(),
            wechat_cdn_base_url: DEFAULT_WECHAT_CDN_BASE_URL.to_string(),
        }
    }
}

fn default_state_path() -> PathBuf {
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path)
            .join("wechat-agent-gateway")
            .join("state.json");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("wechat-agent-gateway")
            .join("state.json");
    }
    PathBuf::from("state.json")
}
