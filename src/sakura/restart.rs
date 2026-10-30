use anyhow::{Result, anyhow};
use std::time::Duration;
use tracing::{info, warn};

// The sakura agent can report "not ready" right after login-er waits; the
// tunnel POST then fails. Restart the sakura services and wait until the
// local agent answers again before retrying the POST.
const READY_BUDGET: Duration = Duration::from_secs(45);
const READY_POLL: Duration = Duration::from_secs(1);

/// Does the local agent answer right now?
async fn agent_ready() -> bool {
    let Ok(agent) = crate::sakura::agent::AgentClient::new() else {
        return false;
    };
    agent.check().await.is_ok()
}

/// Poll until the local agent answers; the budget covers a fresh service
/// start (Sakura can sit "not ready" for a while).
pub async fn wait_ready(budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        if agent_ready().await {
            return true;
        }
        tokio::time::sleep(READY_POLL).await;
    }
    false
}

/// Make the agent usable: wait briefly, restart the sakura services when it
/// does not answer, then wait out the full readiness budget.
pub async fn bring_up_agent() -> Result<()> {
    if agent_ready().await {
        return Ok(());
    }
    info!("Sakura agent is not answering; restarting its services");
    restart().await?;
    if wait_ready(READY_BUDGET).await {
        info!("Sakura agent is ready again");
        return Ok(());
    }
    anyhow::bail!(
        "Sakura agent did not become ready within {}s after a service restart",
        READY_BUDGET.as_secs()
    )
}

/// Restart the Sakura services:
///   1. NGASAK_SAKURA_RESTART_CMD / config [sakura] restart_command wins,
///   2. else platform default (systemd units on Linux, PowerShell on Windows).
pub async fn restart() -> Result<()> {
    let cfg = crate::config::load_config().ok();
    let cfg_cmd = cfg
        .as_ref()
        .and_then(|c| c.sakura.restart_command.clone())
        .filter(|c| !c.trim().is_empty());
    let env_cmd = std::env::var("NGASAK_SAKURA_RESTART_CMD").ok();

    if let Some(cmd) = env_cmd.filter(|c| !c.trim().is_empty()).or(cfg_cmd) {
        info!("Running Sakura restart command: {}", cmd);
        let out = run_shell(cmd).await;
        return match out {
            Ok((text, code)) => {
                if code == 0 {
                    for line in text.lines().filter(|l| !l.trim().is_empty()) {
                        info!("restart: {}", line.trim());
                    }
                    Ok(())
                } else {
                    Err(anyhow!(
                        "restart command exited {}:\n{}",
                        code,
                        text.trim()
                    ))
                }
            }
            Err(e) => Err(anyhow!("restart command failed: {e}")),
        };
    }

    #[cfg(target_os = "linux")]
    return restart_linux().await;
    #[cfg(target_os = "windows")]
    return restart_windows().await;
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    return Err(anyhow!(
        "no built-in Sakura service restart for this platform; set NGASAK_SAKURA_RESTART_CMD"
    ));
}

async fn run_shell(cmd: String) -> std::io::Result<(String, i32)> {
    #[cfg(target_family = "unix")]
    {
        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await?;
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        Ok((text, out.status.code().unwrap_or(-1)))
    }
    #[cfg(target_family = "windows")]
    {
        use tokio::process::Command;
        let out = Command::new("cmd.exe").arg("/D").arg("/C").arg(cmd).output().await?;
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        Ok((text, out.status.code().unwrap_or(-1)))
    }
}

#[cfg(target_os = "linux")]
async fn restart_linux() -> Result<()> {
    // Discover candidate units ("sakura") in both the user session and the
    // system bus. `systemctl` itself decides whether a restart is allowed.
    let scopes: [(&str, Option<&str>); 2] = [("user", Some("--user")), ("system", None)];
    let mut units: Vec<String> = Vec::new();
    for (_, flag) in scopes {
        let mut args: Vec<&str> = Vec::new();
        if let Some(flag) = flag {
            args.push(flag);
        }
        args.extend([
            "list-units",
            "--type=service",
            "--all",
            "--no-pager",
            "--no-legend",
        ]);
        let Ok((text, 0)) = run_process("systemctl", args).await else {
            continue;
        };
        for line in text.lines() {
            let unit = line.split_whitespace().next().unwrap_or("");
            if unit.to_lowercase().contains("sakura") && !units.contains(&unit.to_string()) {
                units.push(unit.to_string());
            }
        }
    }

    if units.is_empty() {
        anyhow::bail!(
            "no systemd unit matching 'sakura' found; set [sakura] restart_command (or NGASAK_SAKURA_RESTART_CMD); sudo rights may be needed for a system unit"
        );
    }

    let mut restarted = Vec::new();
    for unit in &units {
        let attempts: [(&str, Option<&str>); 2] =
            [("user", Some("--user")), ("system", None)];
        for (label, flag) in attempts {
            let mut args: Vec<&str> = Vec::new();
            if let Some(flag) = flag {
                args.push(flag);
            }
            args.push("restart");
            args.push(unit);
            if let Ok((_, 0)) = run_process("systemctl", args).await {
                restarted.push(format!("{unit} ({label})"));
                break;
            }
        }
        if !restarted.iter().any(|r| r.starts_with(unit.as_str())) {
            warn!("failed to restart {unit} (no rights?)");
        }
    }
    if restarted.is_empty() {
        anyhow::bail!("could not restart any of: {}", units.join(", "));
    }
    info!("Restarted: {}", restarted.join(", "));
    Ok(())
}

#[cfg(target_os = "linux")]
async fn run_process<I, S>(program: &str, args: I) -> std::io::Result<(String, i32)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    let out = cmd.output().await?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((text, out.status.code().unwrap_or(-1)))
}

#[cfg(target_os = "windows")]
async fn restart_windows() -> Result<()> {
    // Sakura runs as a Windows service; discover it by name pattern and
    // restart in place. NGASAK_SAKURA_SERVICE narrows it (default: sakura).
    let pattern = std::env::var("NGASAK_SAKURA_SERVICE")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| String::from("sakura"));
    let ps = format!(
        "$svcs = Get-Service | Where-Object {{ $_.Name -like '*{pattern}*' -or $_.DisplayName -like '*{pattern}*' }}; \
if (-not $svcs) {{ 'NONE'; exit 1 }}; \
foreach ($s in $svcs) {{ try {{ Restart-Service -Name $s.Name -Force -ErrorAction Stop; \"restarted:$($s.Name)\" }} catch {{ \"failed:$($s.Name):$($_.Exception.Message)\" }} }}"
    );
    let Ok((text, code)) = run_shell(
        format!("powershell -NoProfile -NonInteractive -Command \"{ps}\""),
    )
    .await
    else {
        anyhow::bail!("powershell could not be started");
    };
    let mut restarted = Vec::new();
    let mut failed = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("restarted:") {
            restarted.push(name.to_string());
        } else if let Some(rest) = line.strip_prefix("failed:") {
            failed.push(rest.to_string());
        }
    }
    for r in &restarted {
        info!("Restarted service {r}");
    }
    for f in &failed {
        warn!("Service restart failed: {f}");
    }
    if code != 0 && restarted.is_empty() {
        if text.contains("NONE") {
            anyhow::bail!(
                "no service matching '{}' found; set NGASAK_SAKURA_SERVICE or NGASAK_SAKURA_RESTART_CMD",
                pattern
            );
        }
        anyhow::bail!("could not restart the Sakura service (admin rights?)");
    }
    Ok(())
}
