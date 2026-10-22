use anyhow::Result;
use clap::{Args, Subcommand};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;
use crate::sakura::autopush::Autopush;

#[derive(Subcommand)]
pub enum Commands {
    /// Run full automation flow (ngate connect -> sakura autopush -> scp login-er)
    Run(RunArgs),
    /// Ngate VPN management
    Ngate(NgateArgs),
    /// Sakura 2FA management
    Sakura(SakuraArgs),
    /// SCP login-er only
    LoginEr(LoginErArgs),
    /// Config management
    Config(ConfigArgs),
}

#[derive(Args)]
pub struct RunArgs {
    #[arg(long, env = "SCP_LOGIN")]
    pub login: Option<String>,
    #[arg(long, env = "SCP_PASS")]
    pub password: Option<String>,
    #[arg(long, default_value_t = 120)]
    pub sakura_wait: u64,
    #[arg(long)]
    pub skip_ngate: bool,
    #[arg(long)]
    pub skip_sakura_push: bool,
}

#[derive(Args)]
pub struct NgateArgs {
    #[command(subcommand)]
    pub command: NgateCommands,
}

#[derive(Subcommand)]
pub enum NgateCommands {
    /// Check ngate status
    Status,
    /// Connect via D-Bus (starts Sakura autopush unless --no-autopush)
    Connect(ConnectArgs),
    /// Connect via GUI (xdotool) - fallback
    ConnectGui,
}

#[derive(Args)]
pub struct ConnectArgs {
    #[arg(long, default_value_t = 120)]
    pub wait: u64,
    #[arg(long)]
    pub no_autopush: bool,
    #[arg(long)]
    pub all_pushes: bool,
}

#[derive(Args)]
pub struct SakuraArgs {
    #[command(subcommand)]
    pub command: SakuraCommands,
}

#[derive(Subcommand)]
pub enum SakuraCommands {
    /// Register this device on the Sakura server
    Register(RegisterArgs),
    /// Ping the Sakura server
    Ping,
    /// Check pending requests
    Check(CheckArgs),
    /// Approve a pending request by guid
    Approve { guid: String },
    /// Deny a pending request by guid
    Deny { guid: String },
    /// Show authentication history
    History,
    /// Poll pending requests and ask for confirmation
    Poll(PollArgs),
    /// Run autopush daemon
    Autopush(AutopushArgs),
    /// Test connection to local agent
    Agent(AgentArgs),
}

#[derive(Args)]
pub struct RegisterArgs {
    #[arg(long)]
    pub server_url: Option<String>,
    #[arg(long)]
    pub server_uuid: Option<String>,
    #[arg(long)]
    pub server_name: Option<String>,
    #[arg(long)]
    pub registry_code: Option<String>,
    #[arg(long)]
    pub qr: Option<String>,
}

#[derive(Args)]
pub struct CheckArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct PollArgs {
    #[arg(long, default_value_t = 5)]
    pub interval: u64,
}

#[derive(Args)]
pub struct AgentArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AutopushArgs {
    #[arg(long, env = "SCP_LOGIN")]
    pub login: Option<String>,
    #[arg(long)]
    pub all: bool,
}

#[derive(Args)]
pub struct LoginErArgs {
    #[arg(long, env = "SCP_LOGIN")]
    pub login: Option<String>,
    #[arg(long, env = "SCP_PASS")]
    pub password: Option<String>,
    #[arg(long)]
    pub skip_ngate_check: bool,
}

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Show current config
    Show,
    /// Initialize/reset config
    Init,
    /// Edit config path
    Path,
}

pub async fn cmd_run(args: RunArgs) -> Result<()> {
    info!("Starting NgaSak full flow");
    let cfg = crate::config::load_config()?;

    let (login, password) = resolve_credentials(args.login, args.password, &cfg)?;

    let online = crate::ngate::status::is_online(&cfg.ngate.binary).await?;
    if args.skip_ngate || online {
        if online {
            info!("Ngate already Online");
        }
    } else {
        let autopush = if args.skip_sakura_push {
            PushFilter::Off
        } else {
            PushFilter::Login(login.clone())
        };
        connect_ngate(&cfg, autopush, args.sakura_wait).await?;
    }

    if cfg.scp.post_delay > 0 {
        info!("Waiting {}s before login-er (post_delay)", cfg.scp.post_delay);
        tokio::time::sleep(std::time::Duration::from_secs(cfg.scp.post_delay)).await;
    }

    crate::scp::login_er::do_login_er(&cfg.scp.login_url, &login, &password).await?;

    Ok(())
}

enum PushFilter {
    Off,
    All,
    Login(String),
}

async fn connect_ngate(cfg: &Config, autopush: PushFilter, wait_secs: u64) -> Result<()> {
    if crate::ngate::status::is_online(&cfg.ngate.binary).await? {
        info!("Ngate already Online");
        return Ok(());
    }

    let token = CancellationToken::new();
    let autopush_handle = match autopush {
        PushFilter::Off => None,
        push_filter => {
            let login = match push_filter {
                PushFilter::Login(login) => Some(login),
                _ => None,
            };
            let mut ap = Autopush::new(
                login,
                std::time::Duration::from_secs_f64(cfg.sakura.poll_interval.max(0.2)),
            )?;
            let token = token.clone();
            Some(tokio::spawn(async move { ap.run_until(token).await }))
        }
    };

    info!("Connecting Ngate via D-Bus...");
    let connect_result = match crate::ngate::dbus::connect().await {
        Ok(_) => Ok(()),
        Err(e) => {
            warn!("D-Bus connect failed: {e:#}. Trying GUI fallback...");
            crate::ngate::dbus::connect_gui_fallback().await
        }
    };

    if let Err(e) = connect_result {
        token.cancel();
        if let Some(handle) = autopush_handle {
            let _ = handle.await;
        }
        return Err(e);
    }

    let mut waited = 0;
    let mut online = false;
    while waited < wait_secs {
        if crate::ngate::status::is_online(&cfg.ngate.binary).await? {
            online = true;
            info!("Ngate Online after {}s", waited);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        waited += 1;
    }

    token.cancel();
    if let Some(handle) = autopush_handle {
        let _ = handle.await;
    }

    if !online {
        anyhow::bail!("Ngate did not come online within {}s", wait_secs);
    }
    Ok(())
}

fn resolve_credentials(
    login: Option<String>,
    password: Option<String>,
    cfg: &Config,
) -> Result<(String, String)> {
    let login = login
        .or_else(|| std::env::var("SCP_LOGIN").ok())
        .or_else(|| cfg.scp.login.clone());
    let password = password.or_else(|| std::env::var("SCP_PASS").ok());

    if let (Some(l), Some(p)) = (login.clone(), password.clone()) {
        return Ok((l, p));
    }

    if let Ok(entry) = keyring::Entry::new("ngasak", "scp") {
        if let Ok(creds) = entry.get_password() {
            let parts: Vec<&str> = creds.splitn(2, ':').collect();
            if parts.len() == 2 {
                return Ok((
                    login.unwrap_or_else(|| parts[0].to_string()),
                    password.unwrap_or_else(|| parts[1].to_string()),
                ));
            }
        }
    }

    let login = match login {
        Some(l) => l,
        None => inquire::Text::new("SCP Login:").prompt()?,
    };
    let password = match password {
        Some(p) => p,
        None => rpassword::prompt_password("SCP Password: ")?,
    };

    if login.trim().is_empty() || password.is_empty() {
        anyhow::bail!("SCP login/password must not be empty");
    }
    Ok((login, password))
}

pub async fn cmd_ngate(args: NgateArgs) -> Result<()> {
    let cfg = crate::config::load_config()?;
    match args.command {
        NgateCommands::Status => {
            let status = crate::ngate::status::get_status(&cfg.ngate.binary).await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        NgateCommands::Connect(connect) => {
            let autopush = if connect.no_autopush {
                PushFilter::Off
            } else if connect.all_pushes {
                PushFilter::All
            } else {
                match cfg
                    .scp
                    .login
                    .clone()
                    .or_else(|| std::env::var("SCP_LOGIN").ok())
                    .filter(|l| !l.trim().is_empty())
                {
                    Some(login) => PushFilter::Login(login),
                    None => {
                        warn!("No expected login configured; approving every push");
                        PushFilter::All
                    }
                }
            };
            connect_ngate(&cfg, autopush, connect.wait).await?;
        }
        NgateCommands::ConnectGui => {
            crate::ngate::dbus::connect_gui_fallback().await?;
            info!("Connect attempted via xdotool");
        }
    }
    Ok(())
}

pub async fn cmd_sakura(args: SakuraArgs) -> Result<()> {
    match args.command {
        SakuraCommands::Register(args) => {
            crate::sakura::register::register(
                args.server_url,
                args.server_uuid,
                args.server_name,
                args.registry_code,
                args.qr,
            )
            .await?;
        }
        SakuraCommands::Ping => {
            let client = crate::sakura::client::SakuraClient::from_config()?;
            let token = client.registry_token();
            let resp = client.ping(token.as_deref()).await?;
            println!("{}", resp);
        }
        SakuraCommands::Check(args) => {
            let client = crate::sakura::client::SakuraClient::from_config()?;
            let resp = client.check_requests().await?;
            if args.json {
                println!("{}", resp);
                return Ok(());
            }
            let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or(serde_json::json!({}));
            let payload = v
                .get("payload")
                .and_then(|p| p.as_array())
                .cloned()
                .unwrap_or_default();
            if payload.is_empty() {
                println!("No pending requests.");
                return Ok(());
            }
            println!("Pending requests: {}", payload.len());
            for (i, item) in payload.iter().enumerate() {
                let guid = item.get("guid").and_then(|g| g.as_str()).unwrap_or("");
                let msg = item.get("message").and_then(|m| m.as_str()).unwrap_or("");
                println!("  [{}] guid={} message={}", i + 1, guid, msg);
            }
        }
        SakuraCommands::Approve { guid } => {
            let client = crate::sakura::client::SakuraClient::from_config()?;
            let resp = client.send_response(&guid, true).await?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            println!("HTTP {} {}", status, body);
        }
        SakuraCommands::Deny { guid } => {
            let client = crate::sakura::client::SakuraClient::from_config()?;
            let resp = client.send_response(&guid, false).await?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            println!("HTTP {} {}", status, body);
        }
        SakuraCommands::History => {
            let client = crate::sakura::client::SakuraClient::from_config()?;
            let resp = client.history().await?;
            println!("{}", resp);
        }
        SakuraCommands::Poll(args) => {
            let client = crate::sakura::client::SakuraClient::from_config()?;
            let mut seen = std::collections::HashSet::new();
            info!("Polling every {}s. Ctrl+C to stop.", args.interval);
            loop {
                if let Ok(resp) = client.check_requests().await {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                        if let Some(payload) = v.get("payload").and_then(|p| p.as_array()) {
                            for item in payload {
                                let Some(guid) = item.get("guid").and_then(|g| g.as_str()) else {
                                    continue;
                                };
                                if seen.contains(guid) {
                                    continue;
                                }
                                seen.insert(guid.to_string());
                                let msg = item
                                    .get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("");
                                println!("\n=== NEW REQUEST ===");
                                println!("guid: {}", guid);
                                println!("message: {}", msg);
                                let ans = inquire::Text::new("Approve? [y/N]:")
                                    .prompt()
                                    .unwrap_or_default();
                                let approve = matches!(
                                    ans.trim().to_lowercase().as_str(),
                                    "y" | "yes" | "д" | "да"
                                );
                                let rr = client.send_response(guid, approve).await?;
                                let st = rr.status();
                                let body = rr.text().await.unwrap_or_default();
                                println!("response: HTTP {} {}", st, body);
                            }
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(args.interval)).await;
            }
        }
        SakuraCommands::Autopush(args) => {
            let cfg = crate::config::load_config()?;
            let login = if args.all {
                None
            } else {
                args.login
                    .or_else(|| std::env::var("SCP_LOGIN").ok())
                    .or(cfg.scp.login.clone())
            };
            if login.is_none() && !args.all {
                warn!("No expected login configured; approving every push (use --all to silence)");
            }
            let ap = Autopush::new(
                login,
                std::time::Duration::from_secs_f64(cfg.sakura.poll_interval.max(0.2)),
            )?;
            ap.run_forever().await?;
        }
        SakuraCommands::Agent(_) => {
            let agent = crate::sakura::agent::AgentClient::new()?;
            let check = agent.check().await?;
            let uid = agent.uid().await?;
            println!("check: {}", check);
            println!("uid_len: {}", uid.len());
        }
    }
    Ok(())
}

pub async fn cmd_login_er(args: LoginErArgs) -> Result<()> {
    let cfg = crate::config::load_config()?;
    let login = args
        .login
        .or_else(|| std::env::var("SCP_LOGIN").ok())
        .or_else(|| cfg.scp.login.clone())
        .unwrap_or_else(|| inquire::Text::new("SCP Login:").prompt().unwrap_or_default());
    let password = args
        .password
        .or_else(|| std::env::var("SCP_PASS").ok())
        .unwrap_or_else(|| rpassword::prompt_password("SCP Password: ").unwrap_or_default());

    if !args.skip_ngate_check && !crate::ngate::status::is_online(&cfg.ngate.binary).await? {
        anyhow::bail!("Ngate not online. Connect first or use --skip-ngate-check");
    }

    crate::scp::login_er::do_login_er(&cfg.scp.login_url, &login, &password).await?;
    Ok(())
}

pub async fn cmd_config(args: ConfigArgs) -> Result<()> {
    match args.command {
        ConfigCommands::Show => {
            let cfg = crate::config::load_config()?;
            println!("{}", toml::to_string_pretty(&cfg)?);
        }
        ConfigCommands::Init => {
            let cfg = Config::default();
            crate::config::save_config(&cfg)?;
            info!("Config created at {:?}", crate::config::config_path()?);
        }
        ConfigCommands::Path => {
            println!("{}", crate::config::config_path()?.display());
        }
    }
    Ok(())
}
