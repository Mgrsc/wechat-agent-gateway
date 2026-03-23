use tracing::info;
use tracing_subscriber::EnvFilter;
use wechat_agent_gateway::api::build_router_with_config;
use wechat_agent_gateway::cli::{Command, run_login_command};
use wechat_agent_gateway::config::RuntimeConfig;
use wechat_agent_gateway::state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("wechat_agent_gateway=info,tower_http=info")),
        )
        .init();

    let command = match Command::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    match command {
        Command::Serve => {
            let config = RuntimeConfig::from_env().expect("load runtime config");
            let state =
                AppState::from_disk(config.state_path.clone()).expect("load persistent state");
            let app = build_router_with_config(state, config.clone());
            let addr = config.listen_addr().expect("parse listen addr");
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .expect("bind tcp listener");

            info!(%addr, "wechat agent gateway listening");

            axum::serve(listener, app).await.expect("serve axum app");
        }
        Command::Login(options) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(error) = run_login_command(options, &mut stdout).await {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
}
