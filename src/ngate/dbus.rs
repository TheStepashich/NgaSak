use anyhow::{Context, Result};
use tracing::{debug, info, warn};
use std::process::Command;
use tokio::time::{sleep, Duration};

#[zbus::proxy(
    interface = "com.canonical.dbusmenu",
    default_service = ":1.23",
    default_path = "/MenuBar"
)]
trait DbusMenu {
    #[zbus(name = "Event")]
    fn event(&self, id: i32, event_id: &str, data: zbus::zvariant::Value<'_>, timestamp: u32) -> zbus::Result<()>;
}

pub async fn find_ngate_bus() -> Result<String> {
    let output = Command::new("busctl")
        .args(["--user", "list", "--no-pager"])
        .output()
        .context("Failed to list D-Bus names")?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut candidates = Vec::new();
    for line in stdout.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 3 && fields[0].starts_with(':') && fields[2].contains("ngate") {
            candidates.push(fields[0].to_string());
        }
    }

    for name in &candidates {
        if has_menu(name) {
            info!("Found ngate at {} with menu", name);
            return Ok(name.clone());
        }
    }

    candidates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("ngateclient not found on D-Bus"))
}

fn has_menu(bus_name: &str) -> bool {
    Command::new("busctl")
        .args([
            "--user", "call", bus_name, "/MenuBar", "com.canonical.dbusmenu", "GetLayout", "iias", "0",
            "--", "-1", "0",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn decode_octal_escapes(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut result: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let c1 = bytes[i + 1];
            let c2 = bytes[i + 2];
            let c3 = bytes[i + 3];
            if c1 >= b'0' && c1 <= b'7' && c2 >= b'0' && c2 <= b'7' && c3 >= b'0' && c3 <= b'7' {
                let val = ((c1 - b'0') as u8) * 64 + ((c2 - b'0') as u8) * 8 + ((c3 - b'0') as u8);
                result.push(val);
                i += 4;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

pub async fn connect() -> Result<()> {
    let cfg = crate::config::load_config()?;
    if crate::ngate::status::is_online(&cfg.ngate.binary).await? {
        info!("Ngate already Online");
        return Ok(());
    }

    let bus_name = find_ngate_bus().await?;
    info!("Using bus: {}", bus_name);

    let conn = zbus::Connection::session().await?;
    let menu = DbusMenuProxy::builder(&conn)
        .destination(bus_name.clone())?
        .path("/MenuBar")?
        .build()
        .await?;

    let mut items = read_menu(&bus_name)?;

    // A stuck/awaiting session shows only "Отключить". Reset it first.
    if find_item(&items, "Подключ").is_none() {
        if let Some(id) = find_item(&items, "Отключ") {
            info!("Ngate in intermediate state; resetting via id={}", id);
            menu.event(id, "clicked", zbus::zvariant::Value::Str("".into()), 0)
                .await?;
            for _ in 0..10 {
                sleep(Duration::from_secs(1)).await;
                items = read_menu(&bus_name)?;
                if find_item(&items, "Подключ").is_some() {
                    break;
                }
            }
        }
    }

    if let Some(id) = find_item(&items, "Подключ")
        .or_else(|| find_item(&items, "connect"))
    {
        info!("Clicking connect: [{}]", id);
        menu.event(id, "clicked", zbus::zvariant::Value::Str("".into()), 0)
            .await?;
        return Ok(());
    }

    warn!("Connect item not found, using fallback id=6");
    menu.event(6, "clicked", zbus::zvariant::Value::Str("".into()), 0)
        .await?;

    Ok(())
}

fn find_item(items: &[(i32, String, bool)], needle: &str) -> Option<i32> {
    items
        .iter()
        .find(|(_, label, enabled)| *enabled && label.contains(needle))
        .map(|(id, _, _)| *id)
}

fn read_menu(bus_name: &str) -> Result<Vec<(i32, String, bool)>> {
    let output = Command::new("busctl")
        .args([
            "--user", "call", bus_name, "/MenuBar", "com.canonical.dbusmenu", "GetLayout", "iias", "0",
            "--", "-1", "0",
        ])
        .output()
        .context("Failed to get layout via busctl")?;
    let layout = String::from_utf8_lossy(&output.stdout).to_string();
    debug!("Layout len: {}", layout.len());

    let mut items: Vec<(i32, String, bool)> = Vec::new();
    let parts: Vec<&str> = layout.split("(ia{sv}av)").collect();
    for part in parts[1..].iter() {
        let trimmed = part.trim();
        if let Some(space_idx) = trimmed.find(' ') {
            let id_str = &trimmed[..space_idx];
            if let Ok(id) = id_str.parse::<i32>() {
                let enabled = trimmed.contains("\"enabled\"") && trimmed.contains("b true");
                let disabled = trimmed.contains("\"enabled\"") && trimmed.contains("b false");
                if let Some(label_start) = trimmed.find("\"label\"") {
                    let rest = &trimmed[label_start..];
                    if let Some(s_start) = rest.find("s \"") {
                        let rest2 = &rest[s_start + 3..];
                        if let Some(s_end) = rest2.find('"') {
                            let label_raw = &rest2[..s_end];
                            let label_decoded = decode_octal_escapes(label_raw);
                            items.push((id, label_decoded, enabled || !disabled));
                        }
                    }
                }
            }
        }
    }

    info!("Menu items:");
    for (id, label, enabled) in &items {
        info!("  [{}] {} (enabled={})", id, label, enabled);
    }

    Ok(items)
}

pub async fn connect_gui_fallback() -> Result<()> {
    warn!("Using GUI fallback with xdotool");
    // Find window
    let output = Command::new("xdotool")
        .args(["search", "--name", "КриптоПро NGate"])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(win) = stdout.lines().next() {
        if !win.is_empty() {
            let _ = Command::new("xdotool").args(["windowactivate", win]).output();
            sleep(Duration::from_secs(2)).await;
            let _ = Command::new("xdotool").args(["key", "--delay", "200", "Return"]).output();
            info!("Sent Enter to NGate window");
            return Ok(());
        }
    }
    anyhow::bail!("NGate window not found")
}
