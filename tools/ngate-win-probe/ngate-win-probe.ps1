<#
ngate-win-probe.ps1 - read-only probe + self-test for Ngate on Windows.

Goal: "test the waters" - establish what the Windows Ngate client looks like on
this machine and whether its console status output can be parsed, BEFORE any
Windows support is written into ngasak.

A default run performs read-only work only (it never starts the client):
  1. locate the Ngate client (PATH, candidate install dirs; -Deep adds a
     PATH-wide search and the uninstall-registry scan)
  2. probe status with  "<binary>" -I   (the same flag ngasak uses on Linux)
  3. show tunnel / service / process hints and the newest client log
  4. show which connect command ngasak would need on Windows

Actions:
  (default) or -Status    the read-only report above
  -SelfTest               parser + probe-shape checks; needs no Ngate install
  -Connect                run the connect command, poll for ONLINE, then try
                          the -GuiCmd fallback and poll again
  -Deep                   also scan PATH-wide (where.exe) and the uninstall registry
  -DryRun                 -Connect without executing the connect command

Inputs -Binary -StatusCmd -ConnectCmd -GuiCmd -StatusArg -Wait, each also
readable from the matching NGASAK_NGATE_* env var (the switch wins). -Quiet
suppresses the report, -Json appends a machine-readable summary.

Exit codes:
  0  ONLINE (or -Connect succeeded, or -SelfTest passed)
  1  found + console probe answered, but not ONLINE
  2  Ngate not found / nothing configured to probe, or no connect command
  3  connect attempted but Ngate never came online
  4  -SelfTest had failures

Examples (cmd.exe):
  powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File ngate-win-probe.ps1
Examples (PowerShell):
  .\ngate-win-probe.ps1
  .\ngate-win-probe.ps1 -SelfTest
  .\ngate-win-probe.ps1 -Connect -ConnectCmd 'ngateclient.exe' -Wait 60
#>

param(
    [switch] $Status,
    [switch] $Connect,
    [switch] $SelfTest,
    [switch] $Deep,
    [switch] $Json,
    [switch] $Quiet,
    [switch] $NoColor,
    [switch] $DryRun,
    [string] $Binary = "",
    [string] $StatusCmd = "",
    [string] $ConnectCmd = "",
    [string] $GuiCmd = "",
    [string] $StatusArg = "-I",
    [int] $Wait = -1
)

# ------------------------------------------------------------------- tables
$script.ExeNames = @(
    "ngateclient.exe",
    "ngate-client.exe",
    "NGateClient.exe",
    "ngate.exe",
    "tbs-cli.exe"
)

# Join-Path chokes on an empty base (non-Windows sessions have no %LOCALAPPDATA%),
# so these paths are built by concatenation instead.
$script.LocalAppData = "$env:LOCALAPPDATA"
$script.AppData = "$env:APPDATA"

$script.CandidateDirs = @(
    "C:\Program Files\NGate",
    "C:\Program Files (x86)\NGate",
    "C:\Program Files (x86)\Krona VPN TBS",
    "C:\Program Files\Krona VPN TBS",
    "C:\NGate",
    "C:\ngate",
    ($script.LocalAppData + "\NGateClient"),
    ($script.LocalAppData + "\ngateclient")
)

$script.LogDirs = @(
    ($script.LocalAppData + "\NGateClient"),
    ($script.LocalAppData + "\ngateclient"),
    ($script.AppData + "\NGateClient"),
    "C:\Program Files (x86)\NGate\logs",
    "C:\Program Files (x86)\NGate\client\logs"
)

$script.FoundBinaries = @()
$script.FoundLogs = @()

# Env var names mirror the ones the (draft) Rust Windows backend reads, so one
# exported set drives both tools.
$script.EnvBinary = "NGASAK_NGATE_BINARY"
$script.EnvStatusCmd = "NGASAK_NGATE_STATUS_CMD"
$script.EnvConnectCmd = "NGASAK_NGATE_CONNECT_CMD"
$script.EnvGuiCmd = "NGASAK_NGATE_GUI_CMD"
$script.EnvWait = "NGASAK_NGATE_WAIT_SECONDS"

$script.OfflineMarkers = @(
    "offline",
    "not online",
    "not connected",
    "disconnected",
    "not configured",
    "unconfigured"
)

$script.OnlineMarkers = @("online", "connected", "registered", "configuring")

# ------------------------------------------------------------------ writers
function Write-Line
{
    param([string] $Message, [string] $Color = "")
    if ($Quiet) { return }
    if ($Color -ne "" -and -not $NoColor)
    {
        Write-Host -Object $Message -ForegroundColor $Color
    }
    else
    {
        Write-Host -Object $Message
    }
}

function Write-Section
{
    param([string] $Title)
    if ($Quiet) { return }
    Write-Line ""
    Write-Line ("=== {0} ===" -f $Title) "Cyan"
}

function Write-Note
{
    param([string] $Message)
    Write-Line ("  {0}" -f $Message)
}

function Write-Hint
{
    param([string] $Message)
    Write-Line ("  .. {0}" -f $Message) "DarkGray"
}

function Show_JsonReport
{
    param($Summary)
    # Emitted even when -Quiet: the machine-readable summary is the point.
    $Summary | ConvertTo-Json -Depth 4 | Write-Output
}

# -------------------------------------------------------------------- util
function Get_EnvironmentValue
{
    param([string] $Name)
    $value = [Environment]::GetEnvironmentVariable($Name)
    if ($null -eq $value) { return "" }
    return "$value"
}

function Test_WindowsSession
{
    # Windows PowerShell (5.1 and older) only runs on Windows. PowerShell 7+
    # has the $IsWindows automatic variable on every platform.
    if ($PSVersionTable.PSVersion.Major -ge 6)
    {
        return [bool](Get-Variable -Name "IsWindows" -ValueOnly -ErrorAction "SilentlyContinue")
    }
    return $true
}

# Online/unknown sniffing. Same table as the Rust status parser is expected to
# use: offline markers win, so "Not Connected" / "not online" / "disconnected"
# can never be matched by the online markers they contain.
function Test_NgateOnlineText
{
    param([string] $Text)

    $lower = "$Text".ToLower()

    foreach ($marker in $script.OfflineMarkers)
    {
        if ($lower.Contains($marker)) { return $false }
    }
    foreach ($marker in $script.OnlineMarkers)
    {
        if ($lower.Contains($marker)) { return $true }
    }
    return $false
}

function Get_FileProductVersion
{
    param([string] $Path)
    try
    {
        $info = [System.Diagnostics.FileVersionInfo]::Get("$Path")
        if ($null -eq $info) { return "" }
        return "$($info.ProductVersion)"
    }
    catch
    {
        return ""
    }
}

# Runs one console command; returns combined stdout+stderr text and exit code.
function Invoke_ConsoleCommand
{
    param([string] $File, [string[]] $Arguments)

    if ($Arguments.Count -eq 0)
    {
        # Refuse to launch an interactive child shell by accident.
        return [pscustomobject]@{ ExitCode = -1; Text = "no arguments: not executed" }
    }

    $lines = @()
    $exitCode = $null

    try
    {
        $raw = & $File @Arguments 2>&1
        $exitCode = $LASTEXITCODE
        foreach ($item in $raw)
        {
            # Native stderr arrives as ErrorRecords; stringifying yields the text.
            $lines += "$item"
        }
    }
    catch
    {
        $lines += "$($_.Exception.Message)"
    }

    if ($null -eq $exitCode) { $exitCode = 0 }

    $separator = [Environment]::NewLine
    return [pscustomobject]@{
        ExitCode = $exitCode
        Text = ($lines -join $separator)
    }
}

# Builds the status probe: an explicit status command wins, otherwise the
# console probe  "<binary>" <StatusArg>.
function Get_StatusProbe
{
    param([string] $Bin, [string] $Cmd, [string] $Arg)

    if ("$Cmd" -ne "")
    {
        return [pscustomobject]@{
            Kind = "status command"
            File = "cmd.exe"
            Arguments = @("/C", "$Cmd")
            Display = "$Cmd"
        }
    }
    if ("$Bin" -eq "") { return $null }

    return [pscustomobject]@{
        Kind = "console probe"
        File = "$Bin"
        Arguments = @("$Arg")
        Display = ('"{0}" {1}' -f $Bin, $Arg)
    }
}

function Get_NgateStatus
{
    param([string] $Bin, [string] $Cmd, [string] $Arg)

    $probe = Get_StatusProbe -Bin $Bin -Cmd $Cmd -Arg $Arg
    if ($null -eq $probe)
    {
        return [pscustomobject]@{ Probed = $false; Online = $false; Raw = ""; ExitCode = -1; Command = "" }
    }

    Write-Line ("  probe: {0}" -f $probe.Display) "DarkGray"

    $result = Invoke_ConsoleCommand -File $probe.File -Arguments $probe.Arguments

    return [pscustomobject]@{
        Probed = $true
        Online = [bool](Test_NgateOnlineText -Text $result.Text)
        Raw = "$($result.Text)".Trim()
        ExitCode = $result.ExitCode
        Command = $probe.Display
    }
}

# ------------------------------------------------------------ discovery ---
# Both collectors append to $script.FoundBinaries / $script.FoundLogs instead of
# returning arrays: a PowerShell function that returns an empty array yields
# $null at the call site, and @($null) then looks like a one-element list.
function Find_NgateBinaries
{
    param([switch] $Deep)

    foreach ($name in $script.ExeNames)
    {
        foreach ($cmd in @(Get-Command -Name $name -ErrorAction "SilentlyContinue"))
        {
            if ($cmd.Source) { $script.FoundBinaries += "$($cmd.Source)" }
        }
    }

    foreach ($dir in $script.CandidateDirs)
    {
        if (-not (Test-Path -LiteralPath $dir)) { continue }
        foreach ($name in $script.ExeNames)
        {
            $path = Join-Path $dir $name
            if (Test-Path -LiteralPath $path) { $script.FoundBinaries += "$path" }
        }
    }

    if ($Deep)
    {
        $stem = ($script.ExeNames[0] -replace "\.exe$", "")
        $scan = Invoke_ConsoleCommand -File "where.exe" -Arguments @("/R", "C:\", "$stem*")
        foreach ($line in ($scan.Text -split "`n"))
        {
            $line = "$line".Trim()
            if ($line -eq "") { continue }
            if (Test-Path -LiteralPath $line -ErrorAction "SilentlyContinue") { $script.FoundBinaries += "$line" }
        }

        $entries = @()
        try
        {
            $entries = @(Get-ItemProperty -Path "HKRM{HKID78876C8-*}\Uninstall\*" -ErrorAction "SilentlyContinue")
        }
        catch
        {
            Write-Hint "registry scan unavailable"
        }
        foreach ($entry in $entries)
        {
            $blob = "$($entry.DisplayName) $($entry.InstallLocation) $($entry.UninstallDirectory)"
            if ($blob -notmatch "(?i)ngate|krona|tbs") { continue }
            Write-Hint ("registry entry DisplayName='{0}' location='{1}'" -f $entry.DisplayName, $entry.InstallLocation)
            foreach ($dir in @("$($entry.InstallLocation)", "$($entry.UninstallDirectory)"))
            {
                if ($dir -eq "" -or -not (Test-Path -LiteralPath $dir)) { continue }
                foreach ($file in @(Get-ChildItem -LiteralPath $dir -Filter "*.exe" -ErrorAction "SilentlyContinue"))
                {
                    if ("$($file.Name)" -match "(?i)ngate|krona|tbs") { $script.FoundBinaries += "$($file.FullName)" }
                }
            }
        }
    }

    $unique = @()
    foreach ($path in $script.FoundBinaries)
    {
        if ($unique -notcontains "$path") { $unique += "$path" }
    }
    $script.FoundBinaries = $unique
}

function Collect_NgateLogs
{
    foreach ($dir in $script.LogDirs)
    {
        if ($dir -eq "" -or -not (Test-Path -LiteralPath $dir)) { continue }
        foreach ($file in @(Get-ChildItem -Path (Join-Path $dir "*.log") -ErrorAction "SilentlyContinue"))
        {
            $script.FoundLogs += $file
        }
    }
}

# --------------------------------------------------------------- reporting -
function Show_Environment
{
    Write-Section "environment"
    Write-Note ("PowerShell {0}; Windows session: {1}" -f $PSVersionTable.PSVersion, (Test_WindowsSession))
    Write-Note ("OS env: {0}" -f $env:OS)
    Write-Note ("ProgramFiles(x86): {0}" -f [Environment]::GetEnvironmentVariable("ProgramFiles(x86)"))
    foreach ($name in @($script.EnvBinary, $script.EnvStatusCmd, $script.EnvConnectCmd, $script.EnvGuiCmd, $script.EnvWait))
    {
        $value = Get_EnvironmentValue -Name $name
        if ($value -eq "") { Write-Hint ("{0} unset" -f $name) }
        else { Write-Note ("{0}={1}" -f $name, $value) }
    }
}

function Show_Discovery
{
    param([switch] $Deep)

    Write-Section "installed clients"
    if ($script.FoundBinaries.Count -eq 0)
    {
        Write-Line "  no Ngate client found on PATH or in the candidate install dirs" "Yellow"
        Write-Hint "install CryptoPro NGate, or set -Binary / NGASAK_NGATE_BINARY to its ngateclient.exe"
        if (-not $Deep) { Write-Hint "re-run with -Deep to also scan PATH-wide and the uninstall registry" }
        return
    }
    Write-Note ("{0} candidate(s):" -f $script.FoundBinaries.Count)
    foreach ($path in $script.FoundBinaries)
    {
        $version = Get_FileProductVersion -Path $path
        if ($version -ne "") { Write-Note ("[{0}] fileVersion={1}" -f $path, $version) }
        else { Write-Note ("[{0}]" -f $path) }
    }
}

function Show_StatusResult
{
    param($Result)

    Write-Section "status"
    if (-not $Result.Probed)
    {
        Write-Line "  cannot probe: no -Binary/NGASAK_NGATE_BINARY and no -StatusCmd/NGASAK_NGATE_STATUS_CMD" "Yellow"
        return
    }

    Write-Note ("probe command: {0}" -f $Result.Command)
    Write-Note ("exit code: {0}" -f $Result.ExitCode)
    if ($Result.Online) { Write-Line "Ngate: ONLINE" "Green" }
    else { Write-Line "Ngate: OFFLINE / unknown" "Yellow" }

    $lines = ("$($Result.Raw)" -split "`n")
    $shown = 0
    foreach ($line in $lines)
    {
        if ("$line".Trim() -eq "") { continue }
        Write-Note ("| {0}" -f "$line".Trim())
        $shown++
        if ($shown -ge 8) { break }
    }
    if ($lines.Count -gt 8) { Write-Hint ("({0} further line(s) suppressed)" -f ($lines.Count - 8)) }
}

function Show_TunnelHints
{
    Write-Section "tunnel / service hints"

    if (Get-Command -Name "Get-NetAdapter" -ErrorAction "SilentlyContinue")
    {
        foreach ($adapter in @(Get-NetAdapter -ErrorAction "SilentlyContinue"))
        {
            $flag = ""
            if ("$($adapter.Name)" -match "(?i)ngate|tap|tun|vpn") { $flag = "   <== candidate Ngate tunnel adapter" }
            Write-Note ("adapter {0} status={1} type={2}{3}" -f $adapter.Name, $adapter.Status, $adapter.MacType, $flag)
        }
    }
    else
    {
        Write-Hint "Get-NetAdapter unavailable (Windows 8+ with NetAdapter cmdlets required)"
    }

    $services = @(Get-Service -Name "*ngate*", "*krona*" -ErrorAction "SilentlyContinue")
    if ($services.Count -eq 0) { Write-Hint "no service named ngate/krona" }
    foreach ($service in $services)
    {
        Write-Note ("service {0} [{1}] status={2}" -f $service.Name, $service.DisplayName, $service.Status)
    }

    $processes = @(Get-Process -Name "*ngate*", "*krona*", "*tbs*" -ErrorAction "SilentlyContinue")
    if ($processes.Count -eq 0) { Write-Hint "no ngate/krona/tbs process running" }
    foreach ($process in $processes)
    {
        Write-Note ("process {0} pid={1}" -f $process.ProcessName, $process.Id)
    }
}

function Show_Logs
{
    Write-Section "client logs"
    if ($script.FoundLogs.Count -eq 0)
    {
        Write-Hint "no *.log in the usual locations (see candidate dirs above)"
        return
    }
    $newest = @($script.FoundLogs | Sort-Object -Descending -Property LastWriteTime | Select-Object -First 1)[0]
    Write-Note ("newest: {0} ({1})" -f $newest.FullName, $newest.LastWriteTime)
    $tail = @(Get-Content -LiteralPath "$($newest.FullName)" -Tail 12 -ErrorAction "SilentlyContinue")
    foreach ($line in $tail)
    {
        Write-Note ("log: {0}" -f "$line".Trim())
    }
}

function Show_ConnectPlan
{
    param([string] $Cmd, [string] $Gui, [int] $Budget)

    Write-Section "what ngasak would need on Windows"
    if ("$Cmd" -eq "")
    {
        Write-Line "  no connect command: set -ConnectCmd / NGASAK_NGATE_CONNECT_CMD" "Yellow"
        Write-Hint "there is no D-Bus menu on Windows, so a console/GUI action replaces the dbus clicks"
        Write-Hint "for example, start the client that brings the tunnel up:"
        Write-Hint '  $env:NGASAK_NGATE_CONNECT_CMD = ''"C:\Program Files (x86)\NGate\ngateclient.exe"'' '
        Write-Hint 'then poll: "<binary>" -I until ONLINE (budget: -Wait seconds, one probe per second)'
    }
    else
    {
        Write-Note ("connect command: {0}" -f $Cmd)
    }
    if ("$Gui" -ne "")
    {
        Write-Note ("GUI fallback command: {0}" -f $Gui)
        Write-Hint "-Connect runs the connect command first, then this fallback if still OFFLINE"
    }
    else
    {
        Write-Hint "no -GuiCmd / NGASAK_NGATE_GUI_CMD: -Connect gets a single stage, no fallback"
    }
    Write-Note ("wait budget: {0}s per stage" -f $Budget)
}

# ------------------------------------------------------------- actions -----
function Invoke_ConnectAttempt
{
    param([string] $Bin, [string] $Cmd, [string] $Arg, [string] $ConnectLine, [string] $Gui, [int] $Budget)

    Write-Section "connect"

    $status = Get_NgateStatus -Bin $Bin -Cmd $Cmd -Arg $Arg
    if ($status.Probed -and $status.Online)
    {
        Write-Line "  already ONLINE, nothing to run" "Green"
        return 0
    }

    if ("$ConnectLine" -eq "")
    {
        Write-Line "  no connect command configured, cannot attempt" "Yellow"
        return 2
    }

    if ($DryRun)
    {
        Write-Line ("  dry-run: would run [{0}]" -f $ConnectLine)
        return 0
    }

    Write-Line ("  stage 1 (connect): {0}" -f $ConnectLine)
    $attempt = Invoke_ConsoleCommand -File "cmd.exe" -Arguments @("/C", "$ConnectLine")
    Write-Note ("connect command exit code: {0}" -f $attempt.ExitCode)
    foreach ($line in ($attempt.Text -split "`n"))
    {
        if ("$line".Trim() -eq "") { continue }
        Write-Note ("| {0}" -f "$line".Trim())
    }

    $waited = Wait_For_Online -Bin $Bin -Cmd $Cmd -Arg $Arg -Budget $Budget
    if ($waited.Probed -and $waited.Online)
    {
        Write-Line ("  ONLINE after {0} probe(s)" -f $waited.Probes) "Green"
        return 0
    }
    if (-not $waited.Probed) { return 2 }

    if ("$Gui" -eq "")
    {
        Write-Line ("  stage 1 did not reach ONLINE ({0} probe(s), budget {1}s); no -GuiCmd fallback configured" -f $waited.Probes, $Budget) "Red"
        return 3
    }

    Write-Line ("  stage 2 (GUI/console fallback): {0}" -f $Gui)
    $guiAttempt = Invoke_ConsoleCommand -File "cmd.exe" -Arguments @("/C", "$Gui")
    Write-Note ("fallback command exit code: {0}" -f $guiAttempt.ExitCode)
    foreach ($line in ($guiAttempt.Text -split "`n"))
    {
        if ("$line".Trim() -eq "") { continue }
        Write-Note ("| {0}" -f "$line".Trim())
    }

    $waited = Wait_For_Online -Bin $Bin -Cmd $Cmd -Arg $Arg -Budget $Budget
    if (-not $waited.Probed) { return 2 }
    if ($waited.Online)
    {
        Write-Line ("  ONLINE after the fallback ({0} probe(s))" -f $waited.Probes) "Green"
        return 0
    }
    Write-Line ("  still OFFLINE after both stages ({0} probe(s), budget {1}s each)" -f $waited.Probes, $Budget) "Red"
    return 3
}

# Polls "<binary>" <StatusArg> (or the status command) once per second.
# Returns { Probed, Online, Probes }.
function Wait_For_Online
{
    param([string] $Bin, [string] $Cmd, [string] $Arg, [int] $Budget)

    $polls = 0
    $deadline = (Get-Date).AddSeconds($Budget)
    while ((Get-Date) -lt $deadline)
    {
        $status = Get_NgateStatus -Bin $Bin -Cmd $Cmd -Arg $Arg
        if (-not $status.Probed)
        {
            Write-Line "  cannot poll: no probe available" "Red"
            return [pscustomobject]@{ Probed = $false; Online = $false; Probes = $polls }
        }
        $polls++
        if ($status.Online)
        {
            return [pscustomobject]@{ Probed = $true; Online = $true; Probes = $polls }
        }
        Write-Line ("  probe {0}: {1}" -f $polls, ($status.Raw -replace "\s+", " ")) "DarkGray"
        Start-Sleep -Seconds 1
    }
    return [pscustomobject]@{ Probed = $true; Online = $false; Probes = $polls }
}

# Parser and probe-shape checks that need no Ngate install: the quickest way
# to verify the probe itself on a Windows host.
function Invoke_SelfTest
{
    Write-Section "self-test"
    $failures = 0

    $cases = @(
        @{ Text = "Ngate Client Status: Online"; Expect = $true },
        @{ Text = "Status: Connected, adapter ngate-tap0"; Expect = $true },
        @{ Text = "registered and online"; Expect = $true },
        @{ Text = "configuring"; Expect = $true },
        @{ Text = "Ngate Client Status: Offline"; Expect = $false },
        @{ Text = "OFFLINE"; Expect = $false },
        @{ Text = "Not Connected"; Expect = $false },
        @{ Text = "not online"; Expect = $false },
        @{ Text = "disconnected"; Expect = $false },
        @{ Text = "usage: ngateclient [-I] [--help]"; Expect = $false },
        @{ Text = ""; Expect = $false }
    )

    foreach ($case in $cases)
    {
        $actual = [bool](Test_NgateOnlineText -Text "$($case.Text)")
        if ($actual -ne $case.Expect)
        {
            Write-Line ("  FAIL text='{0}' expected={1} actual={2}" -f $case.Text, $case.Expect, $actual) "Red"
            $failures++
        }
        else
        {
            Write-Line ("  ok   text='{0}' -> online={1}" -f $case.Text, $actual) "DarkGray"
        }
    }

    $probe = Get_StatusProbe -Bin "C:\x y\ngateclient.exe" -Cmd "" -Arg "-I"
    if ($probe.Display -ne '"C:\x y\ngateclient.exe" -I')
    {
        Write-Line ("  FAIL probe display: [{0}]" -f $probe.Display) "Red"
        $failures++
    }
    else
    {
        Write-Line "  ok   console probe quotes paths containing spaces" "DarkGray"
    }

    $override = Get_StatusProbe -Bin "C:\ignored.exe" -Cmd "my-status.cmd 2>&1" -Arg "-I"
    if ($override.Kind -ne "status command")
    {
        Write-Line "  FAIL status command must win over the binary probe" "Red"
        $failures++
    }
    else
    {
        Write-Line "  ok   status command overrides the binary probe" "DarkGray"
    }

    if ($null -ne (Get_StatusProbe -Bin "" -Cmd "" -Arg "-I"))
    {
        Write-Line "  FAIL no probe expected when nothing is configured" "Red"
        $failures++
    }
    else
    {
        Write-Line "  ok   no probe when nothing is configured" "DarkGray"
    }

    $shape = Invoke_ConsoleCommand -File "cmd.exe" -Arguments @("/C", "echo offline-sentinel")
    if ($shape.Text -notmatch "offline-sentinel")
    {
        Write-Line ("  FAIL console capture: [{0}]" -f $shape.Text) "Red"
        $failures++
    }
    else
    {
        Write-Line "  ok   console capture sees stdout of a child command" "DarkGray"
    }

    Write-Section "self-test result"
    if ($failures -eq 0)
    {
        Write-Line ("  all {0} checks passed" -f ($cases.Count + 4)) "Green"
        return 0
    }
    Write-Line ("  {0} check(s) failed" -f $failures) "Red"
    return 4
}

# ------------------------------------------------------------------- main --
$doStatus = [bool]$Status
$doConnect = [bool]$Connect
$doSelfTest = [bool]$SelfTest
if (-not $doStatus -and -not $doConnect -and -not $doSelfTest) { $doStatus = $true }

if ($Wait -lt 0)
{
    $envWait = Get_EnvironmentValue -Name $script.EnvWait
    if ("$envWait" -match "^\d+$") { $Wait = [int]$envWait }
    else { $Wait = 120 }
}

if ("$Binary" -eq "") { $Binary = Get_EnvironmentValue -Name $script.EnvBinary }
if ("$StatusCmd" -eq "") { $StatusCmd = Get_EnvironmentValue -Name $script.EnvStatusCmd }
if ("$ConnectCmd" -eq "") { $ConnectCmd = Get_EnvironmentValue -Name $script.EnvConnectCmd }
if ("$GuiCmd" -eq "") { $GuiCmd = Get_EnvironmentValue -Name $script.EnvGuiCmd }

$summary = [ordered]@{
    tool = "ngate-win-probe"
    psVersion = "$($PSVersionTable.PSVersion)"
    isWindows = (Test_WindowsSession)
    binary = ""
    statusCommand = ""
    connectCommand = ""
    guiCommand = ""
    online = $false
    exitCode = 2
}

if ($doSelfTest)
{
    $selfTestCode = Invoke_SelfTest
    if (-not $doStatus -and -not $doConnect)
    {
        $summary.exitCode = $selfTestCode
        if ($Json) { Show_JsonReport -Summary $summary }
        exit $selfTestCode
    }
    if ($selfTestCode -ne 0)
    {
        Write-Section "result"
        Write-Line ("  probe self-test failed (exit {0}) - report is unreliable" -f $selfTestCode) "Red"
        exit $selfTestCode
    }
}

Show_Environment

$effectiveBinary = ""
if ("$Binary" -ne "")
{
    if (Test-Path -LiteralPath "$Binary") { $script.FoundBinaries = @("$Binary") }
    else { Write-Hint ("-Binary is not an existing path: {0}" -f $Binary) }
}
if ($script.FoundBinaries.Count -eq 0)
{
    Find_NgateBinaries -Deep:$Deep
}

Show_Discovery -Deep:$Deep

$effectiveBinary = "$Binary"
if ($effectiveBinary -eq "" -and $script.FoundBinaries.Count -gt 0) { $effectiveBinary = "$($script.FoundBinaries[0])" }

$summary.binary = $effectiveBinary
$summary.statusCommand = "$StatusCmd"
$summary.connectCommand = "$ConnectCmd"
$summary.guiCommand = "$GuiCmd"

if ($doStatus)
{
    $status = Get_NgateStatus -Bin $effectiveBinary -Cmd $StatusCmd -Arg $StatusArg
    Show_StatusResult -Result $status
    $summary.online = [bool]$status.Online
}

if (Test_WindowsSession)
{
    Show_TunnelHints
    Collect_NgateLogs
    Show_Logs
}

Show_ConnectPlan -Cmd $ConnectCmd -Gui $GuiCmd -Budget $Wait

if ($doConnect)
{
    $connectCode = Invoke_ConnectAttempt -Bin $effectiveBinary -Cmd $StatusCmd -Arg $StatusArg `
        -ConnectLine $ConnectCmd -Gui $GuiCmd -Budget $Wait
    $summary.exitCode = $connectCode
    Write-Section "result"
    if ($connectCode -eq 0)
    {
        Write-Line "Windows connect path is workable: Ngate reached ONLINE from the console" "Green"
        $summary.online = $true
    }
    else
    {
        Write-Line ("Windows connect path NOT proven (exit {0})" -f $connectCode) "Red"
    }
    if ($Json) { Show_JsonReport -Summary $summary }
    exit $connectCode
}

Write-Section "result"
if ($summary.online)
{
    Write-Line "Ngate ONLINE via the console probe - Windows status path confirmed" "Green"
    $summary.exitCode = 0
}
elseif ($effectiveBinary -eq "" -and "$StatusCmd" -eq "")
{
    Write-Line "Ngate not found here (or it installs under a different name)" "Yellow"
    $summary.exitCode = 2
}
else
{
    Write-Line "Ngate found and the console probe answered, but status is not ONLINE" "Yellow"
    $summary.exitCode = 1
}

if ($Json) { Show_JsonReport -Summary $summary }
exit $summary.exitCode
