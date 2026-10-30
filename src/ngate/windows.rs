use crate::ngate::traits::{BoxFuture, NgateBackend};
use anyhow::{Result, anyhow};
use std::time::Instant;
use tokio::process;
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};

#[cfg(target_family = "windows")]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const CLIENT_NAMES: &[&str] = &["ngateclient.exe", "ngateconsoleclient.exe"];

// The Windows ngateclient.exe has no console flags, so the flow mirrors the
// Linux one: bring the client window up and send Enter (Qt fires the default
// button regardless of keyboard focus), then verify the tunnel through the
// adapter state (the "CP-TAP-Windows Adapter V9" interface goes Up).
const DEFAULT_WINDOW_TITLE: &str = "КриптоПро NGate";

#[derive(Debug)]
pub struct WindowsNgateBackend {
    binary: Option<String>,
    status_arg: String,
    status_command: Option<String>,
    connect_command: Option<String>,
    gui_command: Option<String>,
    window_title: String,
    wait_seconds: u64,
    enter_round_1_secs: u64,
    enter_round_3_secs: u64,
}

impl WindowsNgateBackend {
    pub fn new() -> WindowsNgateBackend {
        Self::from_env()
    }

    // Resolution order: NGASAK_NGATE_BINARY > config.toml ngate.binary (only
    // if the path exists as a Windows file) > auto-discovery.
    pub fn from_env() -> WindowsNgateBackend {
        let cfg = crate::config::load_config().ok();
        let mut binary = std::env::var("NGASAK_NGATE_BINARY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        if binary.is_none() {
            binary = resolve_configured_binary(&cfg);
        }
        if binary.is_none() {
            binary = discover_client();
            if let Some(found) = &binary {
                debug!("NGate client discovered: {}", found);
            }
        }
        let status_arg = std::env::var("NGASAK_NGATE_STATUS_ARG")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| String::from("-I"));
        WindowsNgateBackend {
            binary,
            status_arg,
            status_command: std::env::var("NGASAK_NGATE_STATUS_CMD").ok(),
            connect_command: std::env::var("NGASAK_NGATE_CONNECT_CMD").ok(),
            gui_command: std::env::var("NGASAK_NGATE_GUI_CMD").ok(),
            window_title: std::env::var("NGASAK_NGATE_WINDOW_TITLE")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| String::from(DEFAULT_WINDOW_TITLE)),
            wait_seconds: std::env::var("NGASAK_NGATE_WAIT_SECONDS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(120),
            enter_round_1_secs: std::env::var("NGASAK_NGATE_ENTER_ROUND_1")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10),
            enter_round_3_secs: std::env::var("NGASAK_NGATE_ENTER_ROUND_3")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10),
        }
    }

    fn clone_state(&self) -> WindowsNgateBackend {
        WindowsNgateBackend {
            binary: self.binary.clone(),
            status_arg: self.status_arg.clone(),
            status_command: self.status_command.clone(),
            connect_command: self.connect_command.clone(),
            gui_command: self.gui_command.clone(),
            window_title: self.window_title.clone(),
            wait_seconds: self.wait_seconds,
            enter_round_1_secs: self.enter_round_1_secs,
            enter_round_3_secs: self.enter_round_3_secs,
        }
    }

    // Status with raw output; NGASAK_NGATE_STATUS_CMD wins over the native
    // adapter probe; the binary -I probe is the last resort.
    pub async fn status_detail(&self) -> (bool, Option<String>) {
        query_status(
            self.binary.clone(),
            self.status_arg.clone(),
            self.status_command.clone(),
        )
        .await
    }
}

impl Default for WindowsNgateBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NgateBackend for WindowsNgateBackend {
    fn status(&self) -> BoxFuture<'_, Result<bool>> {
        let state = self.clone_state();
        Box::pin(async move {
            let (online, _) = state.status_detail().await;
            Ok(online)
        })
    }

    fn connect(&self) -> BoxFuture<'_, Result<()>> {
        let state = self.clone_state();
        Box::pin(async move { windows_connect_impl(state).await })
    }

    fn connect_gui_fallback(&self) -> BoxFuture<'_, Result<()>> {
        let state = self.clone_state();
        Box::pin(async move { windows_connect_gui_impl(state).await })
    }
}

// ---------------------------------------------------------------- paths
fn resolve_configured_binary(cfg: &Option<crate::config::Config>) -> Option<String> {
    let configured = cfg
        .as_ref()
        .map(|c| c.ngate.binary.clone())
        .unwrap_or_default();
    let trimmed = configured.trim().to_string();
    // A Unix-style absolute path cannot exist here; Windows would resolve
    // "/opt/..." against the current drive.
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    if std::path::Path::new(&trimmed).is_file() {
        Some(trimmed)
    } else {
        None
    }
}

// Looks for the installed client like tools/ngate-win-probe.ps1 does.
fn discover_client() -> Option<String> {
    let mut dirs: Vec<String> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(base) = std::env::var(var) {
            for sub in ["Crypto Pro\\NGate", "CryptoPro\\NGate", "NGate"] {
                dirs.push(format!("{base}\\{sub}"));
            }
        }
    }
    if let Ok(appdata) = std::env::var("LOCALAPPDATA") {
        dirs.push(format!("{appdata}\\NGateClient"));
        dirs.push(format!("{appdata}\\ngateclient"));
    }
    dirs.push(String::from("C:\\NGate"));
    dirs.push(String::from("C:\\ngate"));

    for dir in dirs {
        for name in CLIENT_NAMES {
            let path = format!("{dir}\\{name}");
            if std::path::Path::new(&path).is_file() {
                return Some(path);
            }
        }
    }
    None
}

// ------------------------------------------------------------------- probe
// Returns (online, raw_output): raw None means nothing is configured to probe.
// A non-zero status-command exit code is not an error; the markers decide.
async fn query_status(
    binary: Option<String>,
    status_arg: String,
    status_command: Option<String>,
) -> (bool, Option<String>) {
    if let Some(cmd) = status_command {
        let (text, code) = run_capture(cmd).await;
        if code != 0 {
            info!("Windows status command exited {}", code);
        }
        return crate::ngate::status::parse_status_output(text);
    }
    // Native adapter probe: the CP-TAP interface belongs to the NGate install
    // and goes Up once the tunnel is established (after Sakura approves the
    // push). Same verdict the PowerShell tool computes with Get-NetAdapter.
    if let Some((online, raw)) = native::adapter_probe() {
        return (online, Some(raw));
    }
    let Some(bin) = binary.filter(|b| !b.trim().is_empty()) else {
        return (false, None);
    };
    let out = process::Command::new(&bin)
        .arg(&status_arg)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
    match out {
        // Client missing or cannot start: offline, nothing to show.
        Err(_) => (false, None),
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            crate::ngate::status::parse_status_output(format!("{stdout}\n{stderr}"))
        }
    }
}

// One probe per second until the Instant deadline. None = no probe configured;
// Some(b) = online state after the budget.
async fn wait_for_online(
    binary: Option<String>,
    status_arg: String,
    status_command: Option<String>,
    wait_seconds: u64,
) -> Option<bool> {
    let deadline = Instant::now() + Duration::from_secs(wait_seconds);
    let mut polls = 0u64;
    while Instant::now() < deadline {
        let (online, raw) =
            query_status(binary.clone(), status_arg.clone(), status_command.clone()).await;
        polls += 1;
        if online {
            info!("Ngate Online after {} probe(s)", polls);
            return Some(true);
        }
        if let Some(r) = raw {
            debug!("Ngate still offline: {}", r.replace('\n', " ").trim());
        } else if polls == 1 {
            return None; // no probe configured at all
        }
        sleep(Duration::from_secs(1)).await;
    }
    Some(false)
}

// --------------------------------------------------------------- connect UX
async fn windows_connect_impl(state: WindowsNgateBackend) -> Result<()> {
    let WindowsNgateBackend {
        binary,
        status_arg,
        status_command,
        connect_command,
        gui_command,
        window_title,
        wait_seconds,
        enter_round_1_secs,
        enter_round_3_secs,
    } = state;

    let (online, _) = query_status(
        binary.clone(),
        status_arg.clone(),
        status_command.clone(),
    )
    .await;
    if online {
        info!("Ngate already Online");
        return Ok(());
    }

    match connect_command {
        Some(cmd) => {
            info!("Running Windows connect command: {}", cmd);
            spawn_detached(cmd);
            match wait_for_online(
                binary.clone(),
                status_arg.clone(),
                status_command.clone(),
                wait_seconds,
            )
            .await
            {
                Some(true) => return Ok(()),
                None => {
                    return Err(anyhow!(
                        "connect command ran, but no status probe is configured to verify it"
                    ));
                }
                Some(false) => {
                    warn!(
                        "Connect command did not bring Ngate online within {}s",
                        wait_seconds
                    );
                }
            }
        }
        None => {
            // Default: native GUI flow. The tunnel also waits for the Sakura
            // 2FA push; ngasak's autopush approves it while we poll.
            info!(
                "No NGASAK_NGATE_CONNECT_CMD; native flow: show the client window and send Enter (title: '{}')",
                window_title
            );
            native_connect_step(binary.as_deref(), &window_title).await?;
            connect_via_enter_rounds(
                binary.clone(),
                status_arg.clone(),
                status_command.clone(),
                &window_title,
                wait_seconds,
                enter_round_1_secs,
                enter_round_3_secs,
            )
            .await?;
            return Ok(());
        }
    }

    if let Some(cmd) = gui_command {
        warn!("Trying Windows GUI fallback command: {}", cmd);
        spawn_detached(cmd);
        match wait_for_online(
            binary.clone(),
            status_arg.clone(),
            status_command.clone(),
            wait_seconds,
        )
        .await
        {
            Some(true) => return Ok(()),
            None => {
                return Err(anyhow!(
                    "GUI fallback ran, but no status probe is configured to verify it"
                ));
            }
            Some(false) => {}
        }
    }

    anyhow::bail!(
        "Ngate did not come online within {}s after each connect attempt",
        wait_seconds,
    );
}

// Enter with an early-check budget: if no push appeared and the tunnel did
// not start within the budget, the Enter most likely never reached the window
// (foreground activation can be refused), so click again at once instead of
// waiting out the whole budget. Round 2 gets the full budget (Sakura push is
// being approved); round 3 is a short reset attempt (mirror of the Linux
// reset-then-connect retry for the stuck "Отключить" state).
async fn connect_via_enter_rounds(
    binary: Option<String>,
    status_arg: String,
    status_command: Option<String>,
    window_title: &str,
    wait_seconds: u64,
    enter_round_1_secs: u64,
    enter_round_3_secs: u64,
) -> Result<()> {
    let rounds = [enter_round_1_secs, wait_seconds, enter_round_3_secs];
    let mut tried: u32 = 0;
    for budget in rounds {
        tried += 1;
        native_click_enter(window_title).await?;
        match wait_for_online(
            binary.clone(),
            status_arg.clone(),
            status_command.clone(),
            budget,
        )
        .await
        {
            Some(true) => return Ok(()),
            None => {
                return Err(anyhow!(
                    "native connect ran, but no status probe is configured to verify it"
                ));
            }
            Some(false) => {
                warn!(
                    "Enter #{} did not bring the tunnel online within {}s",
                    tried, budget
                );
            }
        }
    }
    anyhow::bail!(
        "Ngate did not come online after {} Enter attempts ({}s budget)",
        tried,
        wait_seconds,
    );
}

// Direct GUI path for `ngate connect-gui`.
async fn windows_connect_gui_impl(state: WindowsNgateBackend) -> Result<()> {
    let WindowsNgateBackend {
        binary,
        status_arg,
        status_command,
        gui_command,
        connect_command: _,
        window_title,
        wait_seconds,
        ..
    } = state;

    let (online, _) = query_status(
        binary.clone(),
        status_arg.clone(),
        status_command.clone(),
    )
    .await;
    if online {
        info!("Ngate already Online");
        return Ok(());
    }

    match gui_command.filter(|c| !c.trim().is_empty()) {
        Some(cmd) => {
            info!("Running Windows GUI fallback command: {}", cmd);
            spawn_detached(cmd);
        }
        None => {
            info!(
                "Using the native GUI fallback: show the client window and send Enter (title: '{}')",
                window_title
            );
            native_click_enter(&window_title).await?;
        }
    }

    match wait_for_online(binary, status_arg, status_command, wait_seconds).await {
        Some(true) => Ok(()),
        None => Err(anyhow!(
            "GUI fallback ran, but no status probe is configured to verify it"
        )),
        Some(false) => Err(anyhow!(
            "Ngate did not come online within {}s after GUI fallback",
            wait_seconds,
        )),
    }
}

// Makes sure the client runs (starting it when the binary is known) and its
// window is up. Does NOT send Enter - the rounds in connect_via_enter_rounds
// own the clicks, one Enter per round, so we never fire a double press.
async fn native_connect_step(binary: Option<&str>, window_title: &str) -> Result<()> {
    if native::find_window_by_title(window_title).is_none() {
        let Some(bin) = binary else {
            anyhow::bail!(
                "client window '{}' is not open and no NGate binary is known; open the client, or set NGASAK_NGATE_BINARY / NGASAK_NGATE_CONNECT_CMD",
                window_title
            );
        };
        if !std::path::Path::new(bin).is_file() {
            anyhow::bail!(
                "client window '{}' is not open and the configured binary does not exist: {}",
                window_title,
                bin
            );
        }
        info!("Starting NGate client: {}", bin);
        if let Err(err) = std::process::Command::new(bin).spawn() {
            warn!("NGate client failed to start: {}", err);
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while native::find_window_by_title(window_title).is_none() && Instant::now() < deadline {
            sleep(Duration::from_secs(1)).await;
        }
        if native::find_window_by_title(window_title).is_none() {
            anyhow::bail!(
                "client window '{}' did not appear within 15s after starting the client",
                window_title
            );
        }
    }

    Ok(())
}

// Finds the window, brings it forward and sends Enter.
async fn native_click_enter(window_title: &str) -> Result<()> {
    let Some(hwnd) = native::find_window_by_title(window_title) else {
        anyhow::bail!(
            "client window '{}' not found; start ngateclient.exe or override NGASAK_NGATE_WINDOW_TITLE",
            window_title
        );
    };
    native::activate_window(hwnd);
    // Delay before entering: Qt needs to complete activation/redrawing, otherwise the Enter key will be "swallowed".
    sleep(Duration::from_millis(600)).await;
    native::send_enter();
    info!("Sent Enter to the NGate client window");
    Ok(())
}

// ------------------------------------------------------------- FFI helpers
mod native {
    use tracing::warn;
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, HWND};
    use windows_sys::Win32::Graphics::Gdi::{
        RedrawWindow, RDW_ALLCHILDREN, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
    };
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::System::Threading::AttachThreadInput;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, SetFocus, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_MENU,
        VK_RETURN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOWNA,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn read_wstr(p: *const u16) -> String {
        unsafe {
            let mut len = 0usize;
            while *p.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        }
    }

    pub fn find_window_by_title(title: &str) -> Option<HWND> {
        let t = wide(title);
        let hwnd = unsafe { FindWindowW(std::ptr::null(), t.as_ptr()) };
        if !hwnd.is_null() {
            Some(hwnd)
        } else {
            None
        }
    }

    // Raises the client window to receive keyboard input. Handles the three
    // states it is found in:
    //   - minimized: SW_RESTORE
    //   - hidden (closed to the tray): SW_SHOWNA, no activation flash
    //   - visible: no ShowWindow at all
    // A freshly shown Qt window paints as an empty white frame until it
    // repaints, so an explicit RedrawWindow invalidates the frame and all
    // children right away.
    //
    // Focus stealing: Windows declines plain SetForegroundWindow when the
    // caller is not the foreground process - the Enter then lands in whatever
    // console had focus. The workaround: temporarily join the input queues of
    // our thread and the client thread via AttachThreadInput, plus the classic
    // ALT-key trick when that still is not enough.
    pub fn activate_window(hwnd: HWND) {
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            } else if IsWindowVisible(hwnd) == 0 {
                ShowWindow(hwnd, SW_SHOWNA);
            }
            RedrawWindow(
                hwnd,
                std::ptr::null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME | RDW_UPDATENOW,
            );

            if GetForegroundWindow() == hwnd {
                return; // already focused
            }

            let same_thread = {
                let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
                let client_thread = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
                let joined = fg_thread != 0 && fg_thread != client_thread;
                if joined {
                    AttachThreadInput(fg_thread, client_thread, 1);
                }
                SetForegroundWindow(hwnd);
                if joined {
                    AttachThreadInput(fg_thread, client_thread, 0);
                }
                joined
            };

            // Still not foreground: alternate ALT-press unlocks the switching
            // restriction, one more SetForegroundWindow after it succeeds.
            if GetForegroundWindow() != hwnd {
                if !same_thread {
                    warn!("foreground activation was refused; trying the ALT-key unlock");
                }
                tap_vkey(VK_MENU);
                SetForegroundWindow(hwnd);
                SetFocus(hwnd);
            }
        }
    }

    // A single key down+up burst (used for the ALT foreground unlock).
    fn tap_vkey(vk: u16) {
        let ki_down = KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: 0,
            time: 0,
            dwExtraInfo: 0,
        };
        let ki_up = KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                    ki: ki_down,
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 { ki: ki_up },
            },
        ];
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr() as *mut INPUT,
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    // Enter down/up as one SendInput burst affects whatever window has
    // keyboard focus at that moment (we raise the client right before).
    pub fn send_enter() {
        let ki_down = KEYBDINPUT {
            wVk: VK_RETURN,
            wScan: 0,
            dwFlags: 0,
            time: 0,
            dwExtraInfo: 0,
        };
        let ki_up = KEYBDINPUT {
            wVk: VK_RETURN,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                    ki: ki_down,
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 { ki: ki_up },
            },
        ];
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr() as *mut INPUT,
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    // (online, raw) with ngasak-parser-friendly wording: "online" when the
    // tunnel adapter is Up, "not online" otherwise (offline markers win over
    // the word "online" inside "not online").
    pub fn adapter_probe() -> Option<(bool, String)> {
        let matched = list_adapters_matching()?;
        if matched.is_empty() {
            return None; // no known NGate adapters: cannot judge
        }
        let up: Vec<String> = matched
            .iter()
            .filter(|t| t.2)
            .map(|t| t.0.clone())
            .collect();
        if !up.is_empty() {
            return Some((true, format!("ngate adapters online: {}", up.join(", "))));
        }
        let listing = matched
            .iter()
            .map(|(name, _, is_up)| {
                format!("{}: {}", name, if *is_up { "Up" } else { "disconnected" })
            })
            .collect::<Vec<_>>()
            .join(", ");
        Some((false, format!("ngate adapters not online: {}", listing)))
    }

    fn list_adapters_matching() -> Option<Vec<(String, String, bool)>> {
        const FAMILY_INET: u32 = 2;
        const NO_FLAGS: u32 = 0;
        let mut size: u32 = 15_000;
        for _ in 0..5 {
            let mut buf: Vec<u64> = vec![0; (size as usize / 8) + 1];
            let rc = unsafe {
                GetAdaptersAddresses(
                    FAMILY_INET,
                    NO_FLAGS,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                    &mut size,
                )
            };
            if rc == 0 {
                let mut items = Vec::new();
                let mut cur = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
                while !cur.is_null() {
                    let a = unsafe { &*cur };
                    let name = read_wstr(a.FriendlyName);
                    let desc = read_wstr(a.Description);
                    let is_up = a.OperStatus == IfOperStatusUp;
                    let lower = format!("{} {}", name, desc).to_lowercase();
                    if lower.contains("ngate") || lower.contains("cp-tap") {
                        items.push((name, desc, is_up));
                    }
                    cur = a.Next;
                }
                return Some(items);
            }
            if rc != ERROR_BUFFER_OVERFLOW {
                return None;
            }
        }
        None
    }
}

// ------------------------------------------------------------------ old aux
// Spawns cmd.exe /D /C <cmd> detached: blocking helpers must not stall the
// connect flow; their failures surface as a poll timeout.
fn spawn_detached(cmd: String) {
    let child = std::process::Command::new("cmd.exe")
        .arg("/D")
        .arg("/C")
        .arg(cmd)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    match child {
        Err(err) => warn!("Windows command failed to start: {}", err),
        Ok(_) => {}
    }
}

async fn run_capture(cmd: String) -> (String, i32) {
    let out = process::Command::new("cmd.exe")
        .arg("/D")
        .arg("/C")
        .arg(cmd)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
    match out {
        Err(err) => {
            warn!("Windows status command failed to start: {}", err);
            (String::from(""), -1)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            (format!("{stdout}\n{stderr}"), o.status.code().unwrap_or(-1))
        }
    }
}
