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
# The public half of HQ's release signing key (release/minisign.pub). HQ Lite builds are signed
# with the matching secret key, which only the protected release job can use.
$script:HqSigningKey = 'RWTSi05PPMb9UVVnGilhLWT7h/mjQ1VjfAEXszxJB/Er8UEsCXFc3o1/'

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
        # A build without a signature is never installed.
        $sig = $r.assets | Where-Object { $_.name -eq ($zip.name + '.sha256.minisig') } | Select-Object -First 1
        if (-not $sig) { continue }
        return @{ Tag = $r.tag_name; ZipName = $zip.name; ZipUrl = $zip.browser_download_url; SumUrl = $sum.browser_download_url; SigUrl = $sig.browser_download_url }
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

# Minisign verification (BLAKE2b-512 and Ed25519), because Windows PowerShell 5.1 has neither.
# It checks the signature on the small checksum file; the checksum then vouches for the zip.
# The same code is checked against real minisign output by scripts/windows/install-selftest.ps1.
$script:HqMinisignSource = @'
using System;
using System.Numerics;
using System.Security.Cryptography;
using System.Text;

// Verifies a prehashed minisign signature (BLAKE2b-512 + Ed25519) with nothing but the framework.
// Windows PowerShell 5.1 has neither primitive. Written for the C# 5 compiler it ships with.
public static class HqMinisign
{
    static readonly ulong[] IV = {
        0x6a09e667f3bcc908UL, 0xbb67ae8584caa73bUL, 0x3c6ef372fe94f82bUL, 0xa54ff53a5f1d36f1UL,
        0x510e527fade682d1UL, 0x9b05688c2b3e6c1fUL, 0x1f83d9abfb41bd6bUL, 0x5be0cd19137e2179UL };

    static readonly byte[][] SIGMA = {
        new byte[] { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15 },
        new byte[] { 14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3 },
        new byte[] { 11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4 },
        new byte[] { 7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8 },
        new byte[] { 9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13 },
        new byte[] { 2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9 },
        new byte[] { 12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11 },
        new byte[] { 13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10 },
        new byte[] { 6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5 },
        new byte[] { 10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0 } };

    static ulong Rotr(ulong x, int n) { return (x >> n) | (x << (64 - n)); }

    static void G(ulong[] v, int a, int b, int c, int d, ulong x, ulong y)
    {
        v[a] = v[a] + v[b] + x; v[d] = Rotr(v[d] ^ v[a], 32);
        v[c] = v[c] + v[d];     v[b] = Rotr(v[b] ^ v[c], 24);
        v[a] = v[a] + v[b] + y; v[d] = Rotr(v[d] ^ v[a], 16);
        v[c] = v[c] + v[d];     v[b] = Rotr(v[b] ^ v[c], 63);
    }

    static void Compress(ulong[] h, byte[] block, ulong t, bool last)
    {
        ulong[] m = new ulong[16];
        for (int i = 0; i < 16; i++) m[i] = BitConverter.ToUInt64(block, i * 8);
        ulong[] v = new ulong[16];
        for (int i = 0; i < 8; i++) { v[i] = h[i]; v[i + 8] = IV[i]; }
        v[12] ^= t;
        if (last) v[14] = ~v[14];
        for (int r = 0; r < 12; r++)
        {
            byte[] s = SIGMA[r % 10];
            G(v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
            G(v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
            G(v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
            G(v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
            G(v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
            G(v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
            G(v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
            G(v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
        }
        for (int i = 0; i < 8; i++) h[i] ^= v[i] ^ v[i + 8];
    }

    public static byte[] Blake2b512(byte[] data)
    {
        if (!BitConverter.IsLittleEndian) throw new InvalidOperationException("little-endian only");
        ulong[] h = (ulong[])IV.Clone();
        h[0] ^= 0x01010040UL;
        int pos = 0;
        while (data.Length - pos > 128)
        {
            byte[] blk = new byte[128];
            Buffer.BlockCopy(data, pos, blk, 0, 128);
            pos += 128;
            Compress(h, blk, (ulong)pos, false);
        }
        byte[] fin = new byte[128];
        Buffer.BlockCopy(data, pos, fin, 0, data.Length - pos);
        Compress(h, fin, (ulong)data.Length, true);
        byte[] outb = new byte[64];
        for (int i = 0; i < 8; i++) Buffer.BlockCopy(BitConverter.GetBytes(h[i]), 0, outb, i * 8, 8);
        return outb;
    }

    // ---- Ed25519 (RFC 8032), verification only ----
    static readonly BigInteger P = BigInteger.Pow(2, 255) - 19;
    static readonly BigInteger L = BigInteger.Pow(2, 252) + BigInteger.Parse("27742317777372353535851937790883648493");
    static BigInteger Mod(BigInteger a) { BigInteger r = a % P; return r.Sign < 0 ? r + P : r; }
    static BigInteger Inv(BigInteger a) { return BigInteger.ModPow(Mod(a), P - 2, P); }
    static readonly BigInteger D = Mod(-121665 * Inv(121666));
    static readonly BigInteger SqrtM1 = BigInteger.ModPow(2, (P - 1) / 4, P);

    static BigInteger Le(byte[] b, int off, int len)
    {
        byte[] t = new byte[len + 1];
        Buffer.BlockCopy(b, off, t, 0, len);
        return new BigInteger(t);
    }

    static BigInteger RecoverX(BigInteger y, int sign)
    {
        if (y >= P) return BigInteger.MinusOne;
        BigInteger x2 = Mod((y * y - 1) * Inv(D * y * y + 1));
        if (x2.IsZero) return sign != 0 ? BigInteger.MinusOne : BigInteger.Zero;
        BigInteger x = BigInteger.ModPow(x2, (P + 3) / 8, P);
        if (!Mod(x * x - x2).IsZero) x = Mod(x * SqrtM1);
        if (!Mod(x * x - x2).IsZero) return BigInteger.MinusOne;
        if ((int)(x & 1) != sign) x = P - x;
        return x;
    }

    // Extended coordinates: X, Y, Z, T.
    static BigInteger[] Add(BigInteger[] p, BigInteger[] q)
    {
        BigInteger a = Mod((p[1] - p[0]) * (q[1] - q[0]));
        BigInteger b = Mod((p[1] + p[0]) * (q[1] + q[0]));
        BigInteger c = Mod(2 * p[3] * q[3] * D);
        BigInteger d = Mod(2 * p[2] * q[2]);
        BigInteger e = b - a, f = d - c, g = d + c, h = b + a;
        return new BigInteger[] { Mod(e * f), Mod(g * h), Mod(f * g), Mod(e * h) };
    }

    static BigInteger[] Mul(BigInteger s, BigInteger[] p)
    {
        BigInteger[] q = new BigInteger[] { 0, 1, 1, 0 };
        while (s.Sign > 0)
        {
            if (!(s & 1).IsZero) q = Add(q, p);
            p = Add(p, p);
            s >>= 1;
        }
        return q;
    }

    static bool Equal(BigInteger[] p, BigInteger[] q)
    {
        return Mod(p[0] * q[2] - q[0] * p[2]).IsZero && Mod(p[1] * q[2] - q[1] * p[2]).IsZero;
    }

    static BigInteger[] Decode(byte[] b, int off)
    {
        byte[] t = new byte[32];
        Buffer.BlockCopy(b, off, t, 0, 32);
        int sign = t[31] >> 7;
        t[31] &= 0x7f;
        BigInteger y = Le(t, 0, 32);
        BigInteger x = RecoverX(y, sign);
        if (x.Sign < 0) return null;
        BigInteger[] pt = new BigInteger[] { x, y, 1, Mod(x * y) };
        // A point of small order (a multiple of the cofactor 8) is never a valid key or R.
        BigInteger[] e = Mul(8, pt);
        if (e[0].IsZero) return null;
        return pt;
    }

    static bool Ed25519Verify(byte[] pk, byte[] msg, byte[] sig)
    {
        if (pk.Length != 32 || sig.Length != 64) return false;
        BigInteger[] a = Decode(pk, 0);
        BigInteger[] r = Decode(sig, 0);
        if (a == null || r == null) return false;
        BigInteger s = Le(sig, 32, 32);
        if (s >= L) return false;
        byte[] buf = new byte[64 + msg.Length];
        Buffer.BlockCopy(sig, 0, buf, 0, 32);
        Buffer.BlockCopy(pk, 0, buf, 32, 32);
        Buffer.BlockCopy(msg, 0, buf, 64, msg.Length);
        byte[] dig;
        using (SHA512 sha = SHA512.Create()) dig = sha.ComputeHash(buf);
        BigInteger h = Le(dig, 0, 64) % L;
        BigInteger gy = Mod(4 * Inv(5));
        BigInteger gx = RecoverX(gy, 0);
        BigInteger[] g = new BigInteger[] { gx, gy, 1, Mod(gx * gy) };
        return Equal(Mul(s, g), Add(r, Mul(h, a)));
    }

    static bool Same(byte[] a, int ao, byte[] b, int bo, int n)
    {
        for (int i = 0; i < n; i++) if (a[ao + i] != b[bo + i]) return false;
        return true;
    }

    // Returns null when `minisig` is a valid prehashed signature of `data` by `pubKeyLine`
    // (the base64 line of a minisign .pub file), else a short reason. Never throws.
    public static string Verify(string pubKeyLine, byte[] data, byte[] minisig)
    {
        try
        {
            byte[] pub = Convert.FromBase64String(pubKeyLine.Trim());
            if (pub.Length != 42 || pub[0] != (byte)'E' || pub[1] != (byte)'d') return "the public key is not a minisign key";
            string text = new UTF8Encoding(false, true).GetString(minisig);
            string[] raw = text.Split('\n');
            int n = raw.Length;
            while (n > 0 && raw[n - 1].Trim().Length == 0) n--;
            if (n != 4) return "the signature file is not in the expected format";
            string[] ln = new string[4];
            for (int i = 0; i < 4; i++) ln[i] = raw[i].TrimEnd('\r');
            if (!ln[0].StartsWith("untrusted comment:", StringComparison.Ordinal)) return "the signature file is not in the expected format";
            const string TC = "trusted comment: ";
            if (!ln[2].StartsWith(TC, StringComparison.Ordinal)) return "the signature file is not in the expected format";
            byte[] sig = Convert.FromBase64String(ln[1].Trim());
            byte[] glob = Convert.FromBase64String(ln[3].Trim());
            if (sig.Length != 74 || glob.Length != 64) return "the signature has the wrong size";
            if (sig[0] != (byte)'E' || sig[1] != (byte)'D') return "the signature is not a prehashed minisign signature";
            if (!Same(sig, 2, pub, 2, 8)) return "the signature was made by a different key";
            byte[] pk = new byte[32];
            Buffer.BlockCopy(pub, 10, pk, 0, 32);
            byte[] rs = new byte[64];
            Buffer.BlockCopy(sig, 10, rs, 0, 64);
            if (!Ed25519Verify(pk, Blake2b512(data), rs)) return "the signature does not match the file";
            byte[] tc = Encoding.UTF8.GetBytes(ln[2].Substring(TC.Length));
            byte[] gm = new byte[64 + tc.Length];
            Buffer.BlockCopy(rs, 0, gm, 0, 64);
            Buffer.BlockCopy(tc, 0, gm, 64, tc.Length);
            if (!Ed25519Verify(pk, gm, glob)) return "the signed comment does not match the signature";
            return null;
        }
        catch (Exception)
        {
            return "the signature could not be read";
        }
    }
}
'@

function Initialize-HqMinisign {
    if ('HqMinisign' -as [type]) { return }
    if ($PSVersionTable.PSEdition -eq 'Core') { Add-Type -TypeDefinition $script:HqMinisignSource }
    else { Add-Type -TypeDefinition $script:HqMinisignSource -ReferencedAssemblies 'System.Numerics' }
}

# $null when `Signature` (the bytes of a .minisig file) is a valid HQ release signature over
# `Data`, else a short reason.
function Test-HqSignature {
    param([byte[]]$Data, [byte[]]$Signature, [string]$PublicKey = $script:HqSigningKey)
    # Fail closed: anything but a clean `$null` from the verifier is a failure.
    try {
        Initialize-HqMinisign
        $r = [HqMinisign]::Verify($PublicKey, $Data, $Signature)
    } catch {
        return "the signature check could not run here ($($_.Exception.Message))"
    }
    if ($null -ne $r -and "$r".Length -eq 0) { return 'the signature check gave an unclear answer' }
    return $r
}

# The zip's SHA-256 from a checksum file, trusted only after its signature checks out.
# Throws (so nothing gets installed) when the signature or the file is not right.
function Get-HqVerifiedHash {
    param([string]$SumPath, [string]$SigPath, [string]$ZipName, [string]$PublicKey = $script:HqSigningKey)
    $ErrorActionPreference = 'Stop'
    $sumBytes = [IO.File]::ReadAllBytes($SumPath)
    $why = Test-HqSignature -Data $sumBytes -Signature ([IO.File]::ReadAllBytes($SigPath)) -PublicKey $PublicKey
    if ($why) { throw "this build's signature is not valid ($why); nothing was installed" }
    $want = Get-HqExpectedHash -Text ([Text.Encoding]::ASCII.GetString($sumBytes)) -FileName $ZipName
    if (-not $want) { throw 'the checksum file is not in the expected format' }
    return $want
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
        # The signature comes first: the checksum is trusted only if HQ's release key signed it.
        $sumPath = Join-Path $work ($asset.ZipName + '.sha256')
        $sigPath = Join-Path $work ($asset.ZipName + '.sha256.minisig')
        Get-HqFile -Url $asset.SumUrl -To $sumPath
        Get-HqFile -Url $asset.SigUrl -To $sigPath
        $want = Get-HqVerifiedHash -SumPath $sumPath -SigPath $sigPath -ZipName $asset.ZipName
        Write-HqNote 'Signature OK: signed with the HQ release key.'
        $have = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
        if ($have -ne $want) { throw "the download does not match its checksum (expected $want, got $have)" }
        Write-HqNote 'Checksum matches. The build is signed by HQ, not by a Windows code-signing certificate (Authenticode), so Smart App Control or AppLocker rules that need one can still refuse it.'

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
            Write-HqNote "  2. Ask IT to allow the program by its hash. hq.exe SHA-256: $exeHash (from $($asset.ZipName), $($asset.Tag); signed by HQ, not Authenticode)"
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
