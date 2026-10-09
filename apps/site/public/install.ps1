# HQ installer for Windows. Two editions, your choice:
#
#   Full HQ  inside WSL2: the whole suite, coding agents included. The default where WSL2 works.
#   HQ Lite  one program in your user profile, no WSL2 and no administrator rights. For work
#            computers that block WSL2: the web app, tasks and notes, and VS Code over MCP.
#
#   irm https://agent-hq.online/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://agent-hq.online/install.ps1))) -Edition lite -Yes
#
# Options: -Edition auto|full|lite  -Yes  -DryRun  -InstallDir <path>  -AddToPath  -Version <tag>
# Through `| iex` there are no options; set HQ_EDITION, HQ_YES=1, HQ_DRYRUN=1, HQ_INSTALL_DIR,
# HQ_ADD_TO_PATH=1 or HQ_VERSION first instead.
#
# What it does first is read-only: it looks at whether WSL2 works, whether you are an
# administrator and whether virtualization is on, then recommends an edition and tells you why.
# It never elevates itself (only `wsl --install` asks Windows for approval, on its own), never changes execution policy, security software or system settings,
# and never works around a policy that blocks a program: it says so and stops.
# Works in Windows PowerShell 5.1 and PowerShell 7.

$script:HqRepo = 'CalvinMagezi/hq'
$script:HqLinuxInstaller = 'https://agent-hq.online/install.sh'
$script:HqDocs = 'https://agent-hq.online/install/#windows'

# ─── Decisions (pure: no I/O, so they can be tested) ────────────────────────────────────────────

# Facts about this computer -> which edition to recommend, and why.
#   WslUsable      a WSL2 distribution is installed and answers
#   BuildOk        Windows 10 2004 (19041) or newer, which WSL2 needs
#   Hypervisor     Windows reports a hypervisor is running
#   VirtFirmware   $true / $false when the firmware setting is known, $null when it is not
#   IsAdmin        this session is elevated
function Get-HqRecommendation {
    param($Facts)
    if ($Facts.WslUsable) {
        return @{ Edition = 'full'; Reason = 'WSL2 is already set up here, so you can have the whole suite.' }
    }
    if (-not $Facts.BuildOk) {
        return @{ Edition = 'lite'; Reason = 'This Windows version is too old for WSL2 (it needs Windows 10 version 2004 or newer).' }
    }
    if ($Facts.VirtFirmware -eq $false -and -not $Facts.Hypervisor) {
        return @{ Edition = 'lite'; Reason = 'Virtualization is switched off in this computer''s firmware, and WSL2 needs it.' }
    }
    if (-not $Facts.IsAdmin) {
        return @{ Edition = 'lite'; Reason = 'WSL2 needs an administrator once to turn on, and this session is not one. If your IT team can enable WSL2, Full HQ is the better choice; pick it with -Edition full.' }
    }
    return @{ Edition = 'full'; Reason = 'WSL2 can be set up on this computer, which gives you the whole suite.' }
}

# The newest release that carries a Lite zip and its checksum. `Releases` is the GitHub API list.
function Select-HqLiteAsset {
    param($Releases, [string]$Tag)
    foreach ($r in $Releases) {
        if ($r.draft) { continue }
        if ($Tag -and $r.tag_name -ne $Tag) { continue }
        $zip = $r.assets | Where-Object { $_.name -match '^hq-lite-.+-windows-x86_64\.zip$' } | Select-Object -First 1
        if (-not $zip) { continue }
        $sum = $r.assets | Where-Object { $_.name -eq ($zip.name + '.sha256') } | Select-Object -First 1
        if (-not $sum) { continue }
        return @{ Tag = $r.tag_name; ZipName = $zip.name; ZipUrl = $zip.browser_download_url; SumUrl = $sum.browser_download_url }
    }
    return $null
}

# The hex digest in a `<hex>  <name>` checksum file, or $null when it does not look like one.
function Get-HqExpectedHash {
    param([string]$Text, [string]$FileName)
    foreach ($line in ($Text -split "`n")) {
        if ($line -match '^\s*([0-9a-fA-F]{64})\s+\*?(.+?)\s*$' -and $Matches[2] -eq $FileName) {
            return $Matches[1].ToLowerInvariant()
        }
    }
    return $null
}

# `wsl.exe -l -q` prints UTF-16, which arrives with NUL bytes in between.
function ConvertFrom-HqWslList {
    param([string[]]$Lines)
    $names = @()
    foreach ($l in $Lines) {
        $n = (($l -replace "`0", '') -replace '\(Default\)', '').Trim()
        if ($n -and $n -notmatch '^docker-desktop') { $names += $n }
    }
    return $names
}

# ─── Detection (read-only) ──────────────────────────────────────────────────────────────────────

function Get-HqFacts {
    $f = @{ WslUsable = $false; WslDistro = $null; BuildOk = $true; Hypervisor = $false; VirtFirmware = $null; IsAdmin = $false; LanguageMode = "$($ExecutionContext.SessionState.LanguageMode)" }
    try {
        $id = [Security.Principal.WindowsIdentity]::GetCurrent()
        $f.IsAdmin = ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch { }
    try { $f.BuildOk = ([Environment]::OSVersion.Version.Build -ge 19041) } catch { }
    try {
        $cs = Get-CimInstance Win32_ComputerSystem -ErrorAction Stop
        $f.Hypervisor = [bool]$cs.HypervisorPresent
    } catch { }
    try {
        $cpu = Get-CimInstance Win32_Processor -ErrorAction Stop | Select-Object -First 1
        if ($null -ne $cpu.VirtualizationFirmwareEnabled) { $f.VirtFirmware = [bool]$cpu.VirtualizationFirmwareEnabled }
    } catch { }
    if (Get-Command wsl.exe -ErrorAction SilentlyContinue) {
        try {
            $names = ConvertFrom-HqWslList -Lines @(& wsl.exe -l -q 2>$null)
            # Only distributions running as WSL 2 count (`wsl -l -v` lists the version).
            $v2 = @(& wsl.exe -l -v 2>$null | ForEach-Object { ($_ -replace "`0", '').Trim() } | Where-Object { $_ -match '\s2\s*$' })
            $names = @($names | Where-Object { $n = $_; $v2 | Where-Object { $_ -match [regex]::Escape($n) } })
            if ($names.Count -gt 0) {
                $f.WslDistro = $names[0]
                # A distribution that is listed but cannot start (WSL disabled by policy) fails here.
                $out = & wsl.exe -d $f.WslDistro -- echo hq-ok 2>$null
                $f.WslUsable = ("$out" -replace "`0", '').Trim() -eq 'hq-ok'
            }
        } catch { }
    }
    return $f
}

# ─── Small helpers ──────────────────────────────────────────────────────────────────────────────

function Write-HqStep { param([string]$Text) Write-Host "==> $Text" -ForegroundColor Cyan }
function Write-HqNote { param([string]$Text) Write-Host "    $Text" }

function Confirm-Hq {
    param([string]$Question, [bool]$Default = $true)
    if ($script:HqYes) { return $true }
    # Nobody to ask: do nothing rather than assume yes.
    if (-not [Environment]::UserInteractive) { return $false }
    $hint = if ($Default) { 'Y/n' } else { 'y/N' }
    $a = Read-Host "$Question [$hint]"
    if (-not $a) { return $Default }
    return $a -match '^(y|yes)$'
}

function Initialize-HqNetwork {
    # Windows PowerShell 5.1 negotiates old TLS versions unless asked; the corporate proxy, if any,
    # is the system one with the signed-in user's credentials.
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }
    try {
        $p = [Net.WebRequest]::GetSystemWebProxy()
        $p.Credentials = [Net.CredentialCache]::DefaultNetworkCredentials
        [Net.WebRequest]::DefaultWebProxy = $p
    } catch { }
}

function Get-HqFile {
    param([string]$Url, [string]$To)
    $prev = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'   # the progress bar makes Windows PowerShell 5.1 downloads very slow
    try { Invoke-WebRequest -Uri $Url -OutFile $To -UseBasicParsing -Headers @{ 'User-Agent' = 'hq-install.ps1' } }
    finally { $ProgressPreference = $prev }
}

# ─── HQ Lite ────────────────────────────────────────────────────────────────────────────────────

function Install-HqLite {
    param([string]$Dir, [string]$Tag, [bool]$DryRun, [bool]$AddToPath)

    Write-HqStep "Installing HQ Lite into $Dir"
    Write-HqNote 'No administrator rights, no service, nothing outside your user profile.'
    if ($DryRun) {
        Write-HqNote '(dry run) would download the newest HQ Lite zip, check its SHA-256, unpack it into that folder and run hq.exe --version.'
        return $true
    }

    Initialize-HqNetwork
    try {
        $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$script:HqRepo/releases?per_page=30" -UseBasicParsing -Headers @{ 'User-Agent' = 'hq-install.ps1' }
    } catch {
        $why = if ("$($_.Exception.Message)" -match '403|rate limit') { 'GitHub limits anonymous requests per address (many computers behind one office connection can hit it); try again later.' } else { 'If a company proxy or filter blocks github.com, ask your IT team to allow it.' }
        Write-Warning "Could not get the release list from GitHub: $($_.Exception.Message)"
        Write-HqNote $why
        Write-HqNote 'If a company proxy or filter blocks github.com, ask your IT team to allow it, or download the zip on another computer and unzip it here.'
        return $false
    }
    $asset = Select-HqLiteAsset -Releases $releases -Tag $Tag
    if (-not $asset) {
        Write-Warning 'No published HQ Lite build was found for Windows yet.'
        Write-HqNote "Build it yourself or watch https://github.com/$script:HqRepo/releases. Meanwhile you can use an HQ that runs elsewhere from VS Code with no program installed: see $script:HqDocs"
        return $false
    }
    Write-HqNote "Found $($asset.ZipName) in $($asset.Tag)."

    $work = Join-Path ([IO.Path]::GetTempPath()) ("hq-lite-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $work | Out-Null
    try {
        $zip = Join-Path $work $asset.ZipName
        Get-HqFile -Url $asset.ZipUrl -To $zip
        $sumText = (Invoke-WebRequest -Uri $asset.SumUrl -UseBasicParsing -Headers @{ 'User-Agent' = 'hq-install.ps1' }).Content
        if ($sumText -is [byte[]]) { $sumText = [Text.Encoding]::ASCII.GetString($sumText) }
        $want = Get-HqExpectedHash -Text "$sumText" -FileName $asset.ZipName
        if (-not $want) { throw 'the checksum file is not in the expected format' }
        $have = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
        if ($have -ne $want) { throw "the download does not match its checksum (expected $want, got $have)" }
        Write-HqNote 'Checksum matches. Note: this build is not code-signed yet, so the checksum shows the download is intact, not who made it.'

        $stage = Join-Path $work 'x'
        # Refuse a zip whose entries climb out of the folder (older Expand-Archive does not
        # normalise `..` or absolute names).
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $za = [IO.Compression.ZipFile]::OpenRead($zip)
        try {
            foreach ($e in $za.Entries) {
                if ($e.FullName -match '(^|[\\/])\.\.([\\/]|$)' -or [IO.Path]::IsPathRooted($e.FullName)) { throw "the zip holds an unsafe path: $($e.FullName)" }
            }
        } finally { $za.Dispose() }
        Expand-Archive -Path $zip -DestinationPath $stage -Force
        # The zip holds the files at its top level; accept one wrapper folder as well.
        $root = Get-Item $stage
        if (-not (Test-Path (Join-Path $stage 'hq.exe'))) {
            $inner = Get-ChildItem $stage | Where-Object { $_.PSIsContainer } | Select-Object -First 1
            if ($inner) { $root = $inner }
        }
        if (-not (Test-Path (Join-Path $root.FullName 'hq.exe'))) { throw 'the zip does not contain hq.exe' }

        New-Item -ItemType Directory -Force $Dir | Out-Null
        $exe = Join-Path $Dir 'hq.exe'
        $webDir = Join-Path $Dir 'web'
        $webOld = Join-Path $Dir 'web.old'
        $marker = Join-Path $Dir '.hq-lite'
        # A running hq.exe cannot be overwritten but can be renamed, which is how an update lands.
        $old = "$exe.old"
        $hadExe = Test-Path $exe
        $hadWeb = Test-Path $webDir
        # Never delete a `web` folder that is not ours.
        if ($hadWeb -and -not (Test-Path $marker)) { throw "$webDir exists and was not created by HQ Lite; choose another -InstallDir" }
        if (Test-Path $old) { Remove-Item $old -Force -ErrorAction SilentlyContinue }
        if (Test-Path $webOld) { Remove-Item $webOld -Recurse -Force -ErrorAction SilentlyContinue }
        $swapped = $false
        try {
            if ($hadExe) { Move-Item $exe $old -Force }
            if ($hadWeb) { Move-Item $webDir $webOld -Force }
            $swapped = $true
            Copy-Item (Join-Path $root.FullName 'hq.exe') $exe -Force
            if (Test-Path (Join-Path $root.FullName 'web')) { Copy-Item (Join-Path $root.FullName 'web') $webDir -Recurse -Force }
            Copy-Item (Join-Path $root.FullName 'README.txt') (Join-Path $Dir 'README.txt') -Force -ErrorAction SilentlyContinue
            Set-Content -Path $marker -Value 'created by install.ps1' -ErrorAction SilentlyContinue
        } catch {
            # Put back what was there before, so a failed update never leaves no program at all.
            if ($swapped -or $hadExe) {
                Remove-Item $exe -Force -ErrorAction SilentlyContinue
                if (Test-Path $webDir) { Remove-Item $webDir -Recurse -Force -ErrorAction SilentlyContinue }
                if ($hadExe -and (Test-Path $old)) { Move-Item $old $exe -Force }
                if ($hadWeb -and (Test-Path $webOld)) { Move-Item $webOld $webDir -Force }
            }
            throw
        }
        $exeHash = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()

        # Does this computer let the program run at all? AppLocker, WDAC and Smart App Control
        # decide that, and they only answer when it is started.
        $ran = $false
        $ver = ''
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $ver = (& $exe --version) | Out-String
            $ran = ($LASTEXITCODE -eq 0)
        } catch { $ver = $_.Exception.Message }
        $ErrorActionPreference = $prev
        if (-not $ran) {
            Remove-Item $exe -Force -ErrorAction SilentlyContinue
            if (Test-Path $webDir) { Remove-Item $webDir -Recurse -Force -ErrorAction SilentlyContinue }
            Remove-Item (Join-Path $Dir 'README.txt') -Force -ErrorAction SilentlyContinue
            if ($hadExe -and (Test-Path $old)) { Move-Item $old $exe -Force }
            if ($hadWeb -and (Test-Path $webOld)) { Move-Item $webOld $webDir -Force }
            elseif (-not $hadExe) { Remove-Item $marker -Force -ErrorAction SilentlyContinue }
            Write-Warning "hq.exe would not start here: $($ver.Trim())"
            Write-HqNote 'This usually means a policy on this computer only allows approved programs (AppLocker, Windows Defender Application Control or Smart App Control).'
            Write-HqNote 'HQ does not try to get around that. Your options, lightest first:'
            Write-HqNote "  1. Use an HQ that runs somewhere else from VS Code (nothing to install here): $script:HqDocs"
            Write-HqNote "  2. Ask IT to allow the program by its hash. hq.exe SHA-256: $exeHash (from $($asset.ZipName), $($asset.Tag); unsigned)"
            Write-HqNote '  3. Use Full HQ in WSL2 if your IT team allows it (run this installer again with -Edition full).'
            return $false
        }
        if (Test-Path $webOld) { Remove-Item $webOld -Recurse -Force -ErrorAction SilentlyContinue }
        if (Test-Path $old) { Remove-Item $old -Force -ErrorAction SilentlyContinue }
        Write-HqNote "Installed: $ver"

        if ($AddToPath) {
            $cur = [Environment]::GetEnvironmentVariable('Path', 'User')
            if (-not (($cur -split ';') -contains $Dir)) {
                [Environment]::SetEnvironmentVariable('Path', ($(if ($cur) { "$cur;$Dir" } else { $Dir })), 'User')
                Write-HqNote "Added $Dir to your user PATH (open a new terminal and run hq.exe)."
            }
        } else {
            Write-HqNote "PATH was not changed. Run it as: & '$exe'   (or re-run with -AddToPath)."
        }
        Write-Host ''
        Write-Host 'Next:' -ForegroundColor Green
        Write-Host "  & '$exe' web            open the web app on this computer"
        Write-Host "  & '$exe' mcp install    let VS Code agents use your tasks and notes"
        Write-Host "  & '$exe' task list      the same tasks from a terminal"
        Write-Host "Guide: https://github.com/$script:HqRepo/blob/main/docs/HQ_LITE.md"
        return $true
    } catch {
        Write-Warning "HQ Lite was not installed: $($_.Exception.Message)"
        return $false
    } finally {
        Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Runs a script in a WSL distribution. The script travels base64-encoded, because Windows
# PowerShell 5.1 mangles double quotes inside arguments to native programs.
function Invoke-HqWsl {
    param([string]$Distro, [bool]$AsRoot, [string]$Script)
    $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Script))
    $run = "echo $b64 | base64 -d | bash -l"
    if ($AsRoot) { & wsl.exe -d $Distro -u root -- bash -c $run | Out-Host } else { & wsl.exe -d $Distro -- bash -c $run | Out-Host }
}

# ─── Full HQ (WSL2) ─────────────────────────────────────────────────────────────────────────────

function Install-HqFull {
    param($Facts, [bool]$DryRun)

    Write-HqStep 'Setting up Full HQ in WSL2'
    if (-not $Facts.WslUsable) {
        Write-HqNote 'WSL2 with Ubuntu is not set up on this computer yet.'
        Write-HqNote 'The command that sets it up is:   wsl --install -d Ubuntu'
        Write-HqNote 'Windows asks for administrator approval once, and may ask you to restart.'
        if ($DryRun) { Write-HqNote '(dry run) would offer to run it.'; return $true }
        if (-not (Confirm-Hq 'Run it now?' $true)) {
            Write-HqNote 'Run it yourself when ready, restart if asked, then run this installer again.'
            return $false
        }
        & wsl.exe --install -d Ubuntu | Out-Host
        Write-Host ''
        Write-HqNote 'Restart Windows if it asks, open "Ubuntu" from the Start menu once to create your Linux user, then run this installer again.'
        return $true
    }

    $d = $Facts.WslDistro
    Write-HqNote "Using the WSL2 distribution '$d'."
    $steps = @(
        @{ Text = 'Install what HQ needs in Ubuntu (curl, openssh-server, bubblewrap)'; Root = $true
           Cmd = 'apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y curl openssh-server bubblewrap' },
        @{ Text = 'Turn on systemd (lets the HQ host run as a service) and allow the sandbox''s user namespaces'; Root = $true
           Cmd = 'if ! grep -q "systemd *= *true" /etc/wsl.conf 2>/dev/null; then if grep -q "^\[boot\]" /etc/wsl.conf 2>/dev/null; then sed -i "/^\[boot\]/a systemd=true" /etc/wsl.conf; else printf "[boot]\nsystemd=true\n" >> /etc/wsl.conf; fi; fi; echo "kernel.apparmor_restrict_unprivileged_userns = 0" > /etc/sysctl.d/60-hq.conf' },
        @{ Text = 'Install HQ itself (the Linux installer, inside Ubuntu)'; Root = $false
           Cmd = "curl -fsSL $script:HqLinuxInstaller | bash" }
    )
    foreach ($s in $steps) {
        Write-HqStep $s.Text
        Write-HqNote $s.Cmd
        if ($DryRun) { continue }
        if (-not (Confirm-Hq 'Run this step?' $true)) { Write-HqNote 'Skipped.'; continue }
        Invoke-HqWsl -Distro $d -AsRoot $s.Root -Script $s.Cmd
        if ($LASTEXITCODE -ne 0) { Write-Warning "That step failed (exit code $LASTEXITCODE). Fix it, or finish by hand with docs/WINDOWS.md, then run this installer again."; return $false }
    }
    if (-not $DryRun) {
        Write-Host ''
        Write-Host 'Finish in Ubuntu (these need your input):' -ForegroundColor Green
        Write-Host '  wsl --shutdown          (in PowerShell, once, so systemd starts)'
        Write-Host '  hq install              (in Ubuntu: sets up your notes folder)'
        Write-Host '  hq env                  (add a model key)'
        Write-Host '  hq start all            then open http://localhost:5678 in this browser'
        Write-Host "Guide: https://github.com/$script:HqRepo/blob/main/docs/WINDOWS.md"
    }
    return $true
}

# ─── Entry point ────────────────────────────────────────────────────────────────────────────────

function Install-Hq {
    param([string[]]$Arguments)
    $script:HqYes = ($env:HQ_YES -eq '1')
    $edition = if ($env:HQ_EDITION) { $env:HQ_EDITION } else { 'auto' }
    $dryRun = ($env:HQ_DRYRUN -eq '1')
    $dir = if ($env:HQ_INSTALL_DIR) { $env:HQ_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'hq-lite' }
    $addToPath = ($env:HQ_ADD_TO_PATH -eq '1')
    $tag = $env:HQ_VERSION
    for ($i = 0; $i -lt $Arguments.Count; $i++) {
        switch -Regex ($Arguments[$i]) {
            '^-Edition$'    { $i++; $edition = $Arguments[$i] }
            '^-Yes$'        { $script:HqYes = $true }
            '^-DryRun$'     { $dryRun = $true }
            '^-InstallDir$' { $i++; $dir = $Arguments[$i] }
            '^-AddToPath$'  { $addToPath = $true }
            '^-Version$'    { $i++; $tag = $Arguments[$i] }
            default         { throw "unknown option $($Arguments[$i])" }
        }
    }
    if ($edition -notin @('auto', 'full', 'lite')) { throw "-Edition must be auto, full or lite (got '$edition')" }

    if ("$($ExecutionContext.SessionState.LanguageMode)" -ne 'FullLanguage') {
        Write-Warning 'PowerShell is in a restricted language mode on this computer, so this installer cannot run here.'
        Write-HqNote "Follow the manual steps instead: $script:HqDocs"
        return
    }

    Write-HqStep 'Looking at this computer (nothing is changed yet)'
    $facts = Get-HqFacts
    Write-HqNote ("WSL2 usable: {0}; administrator: {1}; virtualization: {2}" -f $facts.WslUsable, $facts.IsAdmin, $(if ($facts.Hypervisor) { 'running' } elseif ($null -eq $facts.VirtFirmware) { 'unknown' } elseif ($facts.VirtFirmware) { 'on' } else { 'off' }))
    $rec = Get-HqRecommendation -Facts $facts

    if ($edition -eq 'auto') {
        Write-Host ''
        Write-Host "Recommended: $(if ($rec.Edition -eq 'full') { 'Full HQ (WSL2)' } else { 'HQ Lite' })" -ForegroundColor Green
        Write-HqNote $rec.Reason
        Write-Host ''
        Write-Host '  Full HQ   everything: coding agents, chat bots, all tools. Runs inside WSL2.'
        Write-Host '  HQ Lite   web app, tasks, notes and VS Code over MCP. One program, no WSL2, no admin.'
        Write-Host ''
        if ($script:HqYes -or -not [Environment]::UserInteractive) {
            $edition = $rec.Edition
        } else {
            $a = Read-Host "Which one? [F]ull / [L]ite (Enter = $($rec.Edition))"
            $edition = if ($a -match '^[fF]') { 'full' } elseif ($a -match '^[lL]') { 'lite' } else { $rec.Edition }
        }
    }

    # Whatever a function prints is also in its output; its answer is the last item.
    if ($edition -eq 'full') { $res = @(Install-HqFull -Facts $facts -DryRun $dryRun) }
    else { $res = @(Install-HqLite -Dir $dir -Tag $tag -DryRun $dryRun -AddToPath $addToPath) }
    if (-not ($res[-1] -eq $true)) { $global:LASTEXITCODE = 1 }
}

# Dot-sourcing for tests sets HQ_INSTALL_NO_RUN so the functions load without installing.
if ($env:HQ_INSTALL_NO_RUN -ne '1') { Install-Hq -Arguments @($args) }
