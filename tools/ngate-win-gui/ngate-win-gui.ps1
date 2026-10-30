<#
ngate-win-gui.ps1 - drive the GUI-only CryptoPro NGate client on Windows.

The Windows ngateclient.exe exposes no console flags (the -I flag is
Linux-only), so the connect flow from ngasak is mirrored here: find the
client UI and click "Podklyuchit/Connect" - the same way the Linux backend
clicks the tray menu over D-Bus.

Modes (default: -Connect):
  -Connect      ensure the client runs, then try, in order:
                  1. UIA click on a menu/button matching Connect on the client window
                  2. UIA click via the tray icon context menu
                  3. activate the client window and send Enter (Linux parity)
                then poll the status probe: -StatusCmd / NGASAK_NGATE_STATUS_CMD,
                or (fallback) the NGate tunnel adapter state.
  -Status       print online/offline (same probe), exit 0/1/2
  -List         DIAGNOSTIC ONLY: dump client UIA controls, tray buttons,
                windows of the client process and tunnel adapters - run this
                once when the UI Automation guess misses so real names can be
                matched; paste the output back
  -SendEnter    only the Enter fallback

Options come from switches first, then NGASAK_NGATE_* variables:
  NGASAK_NGATE_BINARY        client path (default: auto-discover like the backend)
  NGASAK_NGATE_WAIT_SECONDS  status poll budget, seconds (default 120)
  NGASAK_NGATE_STATUS_CMD    external status command whose output ngasak parses
  NGASAK_NGATE_CONNECT_MENU  extra regex to pick the connect item
  NGASAK_NGATE_WINDOW_TITLE  extra regex for the Enter fallback window

Exit codes: 0 ok / online; 2 no client, no menu found, nothing to probe;
3 connect attempted but not online within budget.
#>

param(
    [switch] $Connect,
    [switch] $Status,
    [switch] $List,
    [switch] $SendEnter,
    [string] $StatusCmd = "",
    [string] $ConnectMenu = "",
    [string] $WindowTitle = "",
    [int] $WaitSeconds = -1
)

$ErrorAction = 'Continue'

# ------------------------------------------------------------- configuration
function EnvVal([string] $Name) {
    $v = [Environment]::GetEnvironmentVariable($Name)
    if ($null -eq $v) { return "" }
    return "$v"
}

# Cyrillic markers go via \uXXXX so the script survives a BOM-less ANSI read
# in Windows PowerShell 5.1 (.NET regex decodes \uXXXX itself).
$script:ConnectMenuRe = '(?i)(\u043F\u043E\u0434\u043A\u043B\u044E\u0447|connect)'
# "podklyuch" -> connect key
if ("$ConnectMenu" -ne "") { $script:ConnectMenuRe = '(?i)' + "$ConnectMenu" }
elseif ("$(EnvVal 'NGASAK_NGATE_CONNECT_MENU')" -ne "") { $script:ConnectMenuRe = '(?i)' + "$(EnvVal 'NGASAK_NGATE_CONNECT_MENU')" }

$script:WindowRe = '(?i)(\u043A\u0440\u0438\u043F\u0442\u043E|ngate)'
# "kripto" -> the client title
if ("$WindowTitle" -ne "") { $script:WindowRe = '(?i)' + "$WindowTitle" }
elseif ("$(EnvVal 'NGASAK_NGATE_WINDOW_TITLE')" -ne "") { $script:WindowRe = '(?i)' + "$(EnvVal 'NGASAK_NGATE_WINDOW_TITLE')" }

if ($WaitSeconds -lt 0) {
    $w = EnvVal 'NGASAK_NGATE_WAIT_SECONDS'
    if ($w -match '^\d+$') { $WaitSeconds = [int]$w } else { $WaitSeconds = 120 }
}

$script:ClientPath = EnvVal 'NGASAK_NGATE_BINARY'
if ("$script:ClientPath" -eq "" -or -not (Test-Path -LiteralPath $script:ClientPath)) {
    $script:ClientPath = ""
    foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:LOCALAPPDATA)) {
        if ("$base" -eq "") { continue }
        foreach ($sub in @('Crypto Pro\NGate', 'CryptoPro\NGate', 'NGate')) {
            $p = Join-Path (Join-Path $base $sub) 'ngateclient.exe'
            if (Test-Path -LiteralPath $p) { $script:ClientPath = $p }
            if ("$script:ClientPath" -ne "") { break }
        }
        if ("$script:ClientPath" -ne "") { break }
    }
}

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$script:Root = [System.Windows.Automation.AutomationElement]::RootElement
$script:TypeButton   = [System.Windows.Automation.ControlType]::Button
$script:TypeMenuItem = [System.Windows.Automation.ControlType]::MenuItem

function New-PropCond($prop, $value) {
    New-Object System.Windows.Automation.PropertyCondition -ArgumentList $prop, $value
}

if (-not ('NgaWin32G' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NgaWin32G {
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
}
'@
}
Add-Type -AssemblyName System.Windows.Forms

# --------------------------------------------------------------- client proc
function Get-ClientProcess {
    @(Get-Process -Name ngateclient -ErrorAction SilentlyContinue)
}

function Ensure-ClientRunning {
    if ("$script:ClientPath" -eq "") {
        Write-Host 'client not running and no path known to start it'
        return $false
    }
    if (@(Get-ClientProcess).Count -gt 0) { return $true }
    Write-Host "starting $script:ClientPath"
    Start-Process -FilePath $script:ClientPath -WindowStyle Minimized | Out-Null
    for ($i = 0; $i -lt 10; $i++) {
        Start-Sleep -Seconds 1
        if (Get-ClientProcess) { return $true }
    }
    return $false
}

function Find-ClientWindows($procIds) {
    $all = @()
    foreach ($id in $procIds) {
        $cond = New-PropCond ([System.Windows.Automation.AutomationElement]::ProcessIdProperty) $id
        $all += @($script:Root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond))
    }
    return $all
}

function Get-NativeHwnd([System.Windows.Automation.AutomationElement] $el) {
    return [IntPtr]($el.Current.NativeWindowHandle)
}

function Show-ClientWindow([System.Windows.Automation.AutomationElement] $Win) {
    $hwnd = Get-NativeHwnd $Win
    if ($hwnd -ne [IntPtr]::Zero) {
        [void][NgaWin32G]::ShowWindowAsync($hwnd, 9) # SW_RESTORE
        [void][NgaWin32G]::SetForegroundWindow($hwnd)
        Start-Sleep -Milliseconds 400
    }
}

# -------------------------------------------------------- strategy 1: UIA UI
function Click-ConnectInApp([System.Windows.Automation.AutomationElement] $Win) {
    foreach ($type in @($script:TypeMenuItem, $script:TypeButton)) {
        $cond = New-PropCond ([System.Windows.Automation.AutomationElement]::ControlTypeProperty) $type
        $items = @($Win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond))
        foreach ($item in $items) {
            $name = $item.Current.Name
            if ("$name" -notmatch $script:ConnectMenuRe) { continue }
            Write-Host "UIA click: [$($type.ProgrammaticName)] '$name'"
            $invoke = $null
            try { $invoke = $item.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern) } catch {}
            if ($invoke) { $invoke.Invoke(); return $true }
            $sel = $null
            try { $sel = $item.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern) } catch {}
            if ($sel) { $sel.Select(); return $true }
        }
    }
    return $false
}

# -------------------------------------------------------- strategy 2: tray UI
function Click-ConnectViaTray {
    $taskbar = $script:Root.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-PropCond ([System.Windows.Automation.AutomationElement]::ClassNameProperty) 'Shell_TrayWnd'))
    if (-not $taskbar) { Write-Host 'Shell_TrayWnd not found'; return $false }

    $cond = New-PropCond ([System.Windows.Automation.AutomationElement]::ControlTypeProperty) $script:TypeButton
    $icon = $null
    foreach ($b in $taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
        if ("$($b.Current.Name)" -match '(?i)\u043A\u0440\u0438\u043F\u0442\u043E|ngate') { $icon = $b; break }
    }
    if (-not $icon) {
        $overflow = $script:Root.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-PropCond ([System.Windows.Automation.AutomationElement]::ClassNameProperty) 'NotifyIconOverflowWindow'))
        if ($overflow) {
            foreach ($b in $overflow.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
                if ("$($b.Current.Name)" -match '(?i)\u043A\u0440\u0438\u043F\u0442\u043E|ngate') { $icon = $b; break }
            }
        }
    }
    if (-not $icon) { Write-Host 'no ngate tray button found'; return $false }

    Write-Host ("tray icon: '{0}'" -f $icon.Current.Name)

    $expand = $null
    try { $expand = $icon.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern) } catch {}
    if ($expand) {
        $expand.Expand()
        Start-Sleep -Milliseconds 800
    } else {
        $invoke = $null
        try { $invoke = $icon.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern) } catch {}
        if ($invoke) { $invoke.Invoke(); Start-Sleep -Seconds 1 }
        else { return $false }
    }

    # a legacy tray menu usually lands in a desktop-level '#32768' window
    $menu = $script:Root.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-PropCond ([System.Windows.Automation.AutomationElement]::ClassNameProperty) '#32768'))
    if (-not $menu) { Write-Host 'no tray menu window appeared'; return $false }
    return (Click-ConnectInApp $menu)
}

# -------------------------------------------------- strategy 3: Enter parity
function Send-EnterFallback {
    $wins = Find-ClientWindows (@(Get-ClientProcess) | ForEach-Object { $_.Id })
    foreach ($w in $wins) {
        if ("$($w.Current.Name)" -notmatch $script:WindowRe) { continue }
        Write-Host ("Enter fallback: activating '{0}'" -f $w.Current.Name)
        Show-ClientWindow $w
        try { $w.SetFocus() } catch {}
        Start-Sleep -Milliseconds 500
        [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
        return $true
    }
    return $false
}

# -------------------------------------------------------------------- status
function Test-OnlineFromText([string] $text) {
    $lower = "$text".ToLower()
    if ($lower -match 'offline|not online|not connected|disconnected|not configured|unconfigured') { return $false }
    if ($lower -match 'online|connected|registered|configuring') { return $true }
    return $false
}

function Get-StatusText {
    $probe = $StatusCmd
    if ("$probe" -eq "") { $probe = EnvVal 'NGASAK_NGATE_STATUS_CMD' }
    if ("$probe" -ne "") {
        $out = cmd.exe /D /C $probe 2>&1
        return [pscustomobject]@{ Probed = $true; Online = (Test-OnlineFromText ($out | Out-String)); Raw = ($out | Out-String).Trim() }
    }
    # no probe configured: fall back to the tunnel adapter state. On the
    # target machine the NGate tunnel rides the "CP-TAP-Windows Adapter V9",
    # so the default regex covers ngate/cp-tap; override with
    # NGASAK_NGATE_ADAPTER when the interface is named differently.
    $adapterRe = '(?i)ngate|cp-tap'
    if ("$(EnvVal 'NGASAK_NGATE_ADAPTER')" -ne "") { $adapterRe = '(?i)' + "$(EnvVal 'NGASAK_NGATE_ADAPTER')" }
    $matched = @(Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object {
        ("$($_.Name) $($_.InterfaceDescription)" -match $adapterRe)
    })
    if ($matched.Count -eq 0) {
        return [pscustomobject]@{ Probed = $false; Online = $false; Raw = 'no adapter matches NGASAK_NGATE_ADAPTER' }
    }
    $up = @($matched | Where-Object { $_.Status -eq 'Up' })
    if ($up.Count -gt 0) {
        return [pscustomobject]@{ Probed = $true; Online = $true; Raw = ($up | ForEach-Object { $_.Name }) -join ', ' + ' online' }
    }
    # known NGate adapters exist but none is Up yet: keep polling
    return [pscustomobject]@{ Probed = $true; Online = $false; Raw = ($matched | ForEach-Object { "$($_.Name):$($_.Status)" }) -join ',' }
}

function Wait-Online {
    $deadline = (Get-Date).AddSeconds($WaitSeconds)
    $polls = 0
    while ((Get-Date) -lt $deadline) {
        $s = Get-StatusText
        $polls++
        if (-not $s.Probed) {
            Write-Host ("cannot probe status: {0}" -f $s.Raw)
            return 2
        }
        if ($s.Online) {
            Write-Host ("ONLINE after {0} probe(s)" -f $polls)
            return 0
        }
        Start-Sleep -Seconds 1
    }
    Write-Host ("still OFFLINE after {0} probe(s)" -f $polls)
    return 3
}

# ----------------------------------------------------------------- diagnostics
function Invoke-List {
    Write-Host '=== client process / windows ==='
    $procs = @(Get-ClientProcess)
    if ($procs.Count -eq 0) { Write-Host '  ngateclient.exe is not running' }
    foreach ($p in $procs) {
        Write-Host ("pid {0}  path {1}" -f $p.Id, $p.Path)
        foreach ($w in (Find-ClientWindows @($p.Id))) {
            Write-Host ("  window '{0}' class {1} hwnd {2}" -f $w.Current.Name, $w.Current.ClassName, (Get-NativeHwnd $w))
        }
    }
    Write-Host '=== UIA controls of the client windows ==='
    foreach ($p in $procs) {
        foreach ($w in (Find-ClientWindows @($p.Id))) {
            Show-ClientWindow $w
            Start-Sleep -Milliseconds 600
            Write-Host ("--- '{0}' ---" -f $w.Current.Name)
            foreach ($el in $w.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
                $canInvoke = $false
                try { $null = $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern); $canInvoke = $true } catch {}
                Write-Host ("  [{0}]{1} '{2}'" -f $el.Current.ControlType.ProgrammaticName, (& { if ($canInvoke) { '*' } else { ' ' } }), $el.Current.Name)
            }
        }
    }
    Write-Host '=== tray buttons (showing all; look for the ngate one) ==='
    $taskbar = $script:Root.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-PropCond ([System.Windows.Automation.AutomationElement]::ClassNameProperty) 'Shell_TrayWnd'))
    if ($taskbar) {
        foreach ($b in $taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, (New-PropCond ([System.Windows.Automation.AutomationElement]::ControlTypeProperty) $script:TypeButton))) {
            Write-Host ("  '{0}' class={1}" -f $b.Current.Name, $b.Current.ClassName)
        }
    }
    Write-Host '=== tunnel adapters ==='
    Get-NetAdapter -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host ("  {0}  status={1}  desc={2}" -f $_.Name, $_.Status, $_.InterfaceDescription)
    }
    return 0
}

# ------------------------------------------------------------------- connect
function Invoke-ConnectFlow {
    if (-not (Ensure-ClientRunning)) { return 2 }

    $wins = Find-ClientWindows (@(Get-ClientProcess) | ForEach-Object { $_.Id })
    $clicked = $false

    Write-Host ('strategy 1: UIA controls inside the client window')
    foreach ($w in $wins) {
        if ("$($w.Current.Name)" -notmatch $script:WindowRe) { continue }
        # Qt apps may only populate their UIA tree once the window is shown
        Show-ClientWindow $w
        if (Click-ConnectInApp $w) { $clicked = $true; break }
    }

    if (-not $clicked) {
        Write-Host 'strategy 2: tray icon context menu'
        if (Click-ConnectViaTray) { $clicked = $true }
    }

    if (-not $clicked) {
        Write-Host 'strategy 3: activate window + Enter'
        if (Send-EnterFallback) { $clicked = $true }
    }

    if (-not $clicked) {
        Write-Host 'no connect strategy worked; run with -List and send the output back'
        return 3
    }

    return (Wait-Online)
}

if ($List) { exit (Invoke-List) }

if ($Status -and -not $Connect) {
    $s = Get-StatusText
    # Machine-readable line: written via Write-Output (Write-Host does not
    # survive stdout redirection in Windows PowerShell 5.1) and worded so the
    # ngasak marker parser cannot mistake "False" for online.
    if ($s.Online) { Write-Output 'ngate status: online' }
    elseif ($s.Probed) { Write-Output "ngate status: offline ($($s.Raw))" }
    else { Write-Output "ngate status: not known ($($s.Raw))" }
    if ($s.Online) { exit 0 }
    if ($s.Probed) { exit 1 }
    exit 2
}

if ($SendEnter) {
    if (Send-EnterFallback) { exit 0 }
    exit 2
}

exit (Invoke-ConnectFlow)
