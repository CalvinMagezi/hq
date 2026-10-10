# Checks the decisions in apps/site/public/install.ps1 without installing anything.
# Run it in Windows PowerShell 5.1 and in PowerShell 7:
#   powershell -NoProfile -File scripts/windows/install-selftest.ps1
#   pwsh -NoProfile -File scripts/windows/install-selftest.ps1
$ErrorActionPreference = 'Stop'
$env:HQ_INSTALL_NO_RUN = '1'
. (Join-Path $PSScriptRoot '..\..\apps\site\public\install.ps1')

$script:failed = 0
function Check([string]$Name, $Actual, $Expected) {
    if ("$Actual" -ne "$Expected") { Write-Host "FAIL $Name : expected '$Expected', got '$Actual'"; $script:failed++ }
    else { Write-Host "ok   $Name" }
}
function Facts($o) {
    $f = @{ WslUsable = $false; BuildOk = $true; Hypervisor = $false; VirtFirmware = $null; IsAdmin = $false }
    foreach ($k in $o.Keys) { $f[$k] = $o[$k] }
    return $f
}

# Recommendation: Full wherever WSL2 works or can be turned on, Lite where it cannot.
Check 'wsl works -> full'            (Get-HqRecommendation (Facts @{ WslUsable = $true })).Edition 'full'
Check 'wsl works, not admin -> full' (Get-HqRecommendation (Facts @{ WslUsable = $true })).Edition 'full'
Check 'old windows -> lite'          (Get-HqRecommendation (Facts @{ BuildOk = $false; IsAdmin = $true })).Edition 'lite'
Check 'virtualization off -> lite'   (Get-HqRecommendation (Facts @{ VirtFirmware = $false; IsAdmin = $true })).Edition 'lite'
Check 'firmware off but hypervisor running -> full' (Get-HqRecommendation (Facts @{ VirtFirmware = $false; Hypervisor = $true; IsAdmin = $true })).Edition 'full'
Check 'no admin, no wsl -> lite'     (Get-HqRecommendation (Facts @{ IsAdmin = $false })).Edition 'lite'
Check 'admin, no wsl yet -> full'    (Get-HqRecommendation (Facts @{ IsAdmin = $true })).Edition 'full'
Check 'unknown firmware, admin -> full' (Get-HqRecommendation (Facts @{ VirtFirmware = $null; IsAdmin = $true })).Edition 'full'
Check 'every answer explains itself' ((Get-HqRecommendation (Facts @{})).Reason.Length -gt 10) $true

# Picking the release asset.
function Rel($tag, $draft, $names) {
    [pscustomobject]@{ tag_name = $tag; draft = $draft; assets = @($names | ForEach-Object { [pscustomobject]@{ name = $_; browser_download_url = "https://example.test/$tag/$_" } }) }
}
$zip = 'hq-lite-0.9.1-abc1234-windows-x86_64.zip'
$rels = @(
    (Rel 'v4' $true @($zip, "$zip.sha256", "$zip.sha256.minisig")),
    (Rel 'v3' $false @('hq-0.9.1-linux-x86_64.tar.gz')),
    (Rel 'v2' $false @($zip)),
    (Rel 'v1' $false @($zip, "$zip.sha256")),
    (Rel 'v0' $false @($zip, "$zip.sha256", "$zip.sha256.minisig"))
)
$pick = Select-HqLiteAsset -Releases $rels
Check 'skips drafts, releases without a lite zip, without a checksum, and without a signature' $pick.Tag 'v0'
Check 'the asset urls are carried' $pick.SumUrl "https://example.test/v0/$zip.sha256"
Check 'the signature url is carried' $pick.SigUrl "https://example.test/v0/$zip.sha256.minisig"
Check 'an unsigned tag is refused even when named' (Select-HqLiteAsset -Releases $rels -Tag 'v1') $null
Check 'nothing found -> null' (Select-HqLiteAsset -Releases @()) $null

# Checksum file parsing, as written by the Windows Lite workflow (LF) and by sha256sum (CRLF-tolerant).
$h = ('ab' * 32)
Check 'hash is read'            (Get-HqExpectedHash -Text "$h  $zip`n" -FileName $zip) $h
Check 'hash is lowercased'      (Get-HqExpectedHash -Text ("AB" * 32 + "  $zip`r`n") -FileName $zip) $h
Check 'binary-mode star'        (Get-HqExpectedHash -Text "$h *$zip`n" -FileName $zip) $h
Check 'wrong file name -> null' (Get-HqExpectedHash -Text "$h  other.zip`n" -FileName $zip) $null
Check 'garbage -> null'         (Get-HqExpectedHash -Text "not a checksum" -FileName $zip) $null

# Signatures. The fixtures were made by real minisign (scripts/windows/make-minisign-fixtures.sh);
# the verifier here is the one install.ps1 carries, so these also compare it with minisign.
$fx = Join-Path $PSScriptRoot 'fixtures'
function Fx([string]$Name) { return [IO.File]::ReadAllBytes((Join-Path $fx $Name)) }
function KeyOf([string]$Name) { return ((Get-Content (Join-Path $fx $Name))[1]).Trim() }
function Utf8([byte[]]$B) { return [Text.Encoding]::UTF8.GetString($B) }
function Bytes([string]$T) { return [Text.Encoding]::UTF8.GetBytes($T) }
$testKey = KeyOf 'test.pub'
$fxZip = 'hq-lite-0.0.0-abc1234-windows-x86_64.zip'
$sum = Fx "$fxZip.sha256"
$sig = Fx "$fxZip.sha256.minisig"
$blob = Fx 'blob.bin'
$blobSig = Fx 'blob.bin.minisig'

Check 'the embedded key is the release key'     $script:HqSigningKey ((Get-Content (Join-Path $PSScriptRoot '..\..\release\minisign.pub'))[1]).Trim()
Check 'a real release signature verifies'        (Test-HqSignature -Data (Fx 'release-manifest.json') -Signature (Fx 'release-manifest.json.minisig')) $null
Check 'a valid signature verifies'               (Test-HqSignature -Data $sum -Signature $sig -PublicKey $testKey) $null
Check 'a multi-block file verifies'              (Test-HqSignature -Data $blob -Signature $blobSig -PublicKey $testKey) $null
Check 'CRLF line endings in the .minisig are fine' (Test-HqSignature -Data $sum -Signature (Bytes ((Utf8 $sig) -replace "`n", "`r`n")) -PublicKey $testKey) $null
$flipped = [byte[]]$sum.Clone(); $flipped[3] = $flipped[3] -bxor 1
Check 'changed data fails'                       ((Test-HqSignature -Data $flipped -Signature $sig -PublicKey $testKey) -ne $null) $true
$blobFlipped = [byte[]]$blob.Clone(); $blobFlipped[900] = $blobFlipped[900] -bxor 1
Check 'changed data deep in a long file fails'   ((Test-HqSignature -Data $blobFlipped -Signature $blobSig -PublicKey $testKey) -ne $null) $true
Check 'another key fails'                        ((Test-HqSignature -Data $sum -Signature $sig -PublicKey (KeyOf 'other.pub')) -ne $null) $true
Check 'the release key does not accept a test signature' ((Test-HqSignature -Data $sum -Signature $sig) -ne $null) $true
$lines = (Utf8 $sig) -split "`n"
$legacy = [Convert]::FromBase64String($lines[1]); $legacy[1] = [byte][char]'d'
$legacyText = ($lines[0], [Convert]::ToBase64String($legacy), $lines[2], $lines[3]) -join "`n"
Check 'a non-prehashed (Ed) signature is refused' ((Test-HqSignature -Data $sum -Signature (Bytes $legacyText) -PublicKey $testKey) -ne $null) $true
$bits = [Convert]::FromBase64String($lines[1]); $bits[30] = $bits[30] -bxor 1
$bitsText = ($lines[0], [Convert]::ToBase64String($bits), $lines[2], $lines[3]) -join "`n"
Check 'a changed signature fails'                ((Test-HqSignature -Data $sum -Signature (Bytes $bitsText) -PublicKey $testKey) -ne $null) $true
$glob = [Convert]::FromBase64String($lines[3]); $glob[7] = $glob[7] -bxor 1
$globText = ($lines[0], $lines[1], $lines[2], [Convert]::ToBase64String($glob)) -join "`n"
Check 'a changed global signature fails'         ((Test-HqSignature -Data $sum -Signature (Bytes $globText) -PublicKey $testKey) -ne $null) $true
$tcText = ($lines[0], $lines[1], ($lines[2] -replace 'fixture', 'fixturf'), $lines[3]) -join "`n"
Check 'a changed trusted comment fails'          ((Test-HqSignature -Data $sum -Signature (Bytes $tcText) -PublicKey $testKey) -ne $null) $true
Check 'a truncated signature fails'              ((Test-HqSignature -Data $sum -Signature $sig[0..60] -PublicKey $testKey) -ne $null) $true
Check 'an empty signature fails'                 ((Test-HqSignature -Data $sum -Signature ([byte[]]@()) -PublicKey $testKey) -ne $null) $true
Check 'extra lines after the signature fail'     ((Test-HqSignature -Data $sum -Signature (Bytes ((Utf8 $sig) + "junk`n")) -PublicKey $testKey) -ne $null) $true
Check 'a bad public key fails, not throws'       ((Test-HqSignature -Data $sum -Signature $sig -PublicKey 'AAAA') -ne $null) $true

# The step the installer runs on a download: signature first, then the hash it vouches for.
$work = Join-Path ([IO.Path]::GetTempPath()) ('hq-selftest-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
try {
    $sp = Join-Path $work 'a.sha256'; $gp = Join-Path $work 'a.sha256.minisig'
    [IO.File]::WriteAllBytes($sp, $sum); [IO.File]::WriteAllBytes($gp, $sig)
    Check 'a signed checksum gives its hash'     (Get-HqVerifiedHash -SumPath $sp -SigPath $gp -ZipName $fxZip -PublicKey $testKey) 'e90644f22e57a29e73a60617c902a1d38095f20bac570540f3364954208cf2b3'
    $thrown = $false; try { Get-HqVerifiedHash -SumPath $sp -SigPath $gp -ZipName 'other.zip' -PublicKey $testKey | Out-Null } catch { $thrown = $true }
    Check 'a signed checksum for another file is refused' $thrown $true
    [IO.File]::WriteAllBytes($sp, $flipped)
    $thrown = $false; try { Get-HqVerifiedHash -SumPath $sp -SigPath $gp -ZipName $fxZip -PublicKey $testKey | Out-Null } catch { $thrown = $true }
    Check 'a tampered checksum file is refused'  $thrown $true
} finally { Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue }

# wsl.exe -l -q output: UTF-16 with NULs, a default marker, and Docker's helper distros.
$lines = @("U`0b`0u`0n`0t`0u`0", "d`0o`0c`0k`0e`0r`0-`0d`0e`0s`0k`0t`0o`0p`0", "", "Debian (Default)")
Check 'wsl list is cleaned' ((ConvertFrom-HqWslList -Lines $lines) -join ',') 'Ubuntu,Debian'

# The whole script in dry-run mode changes nothing and exits cleanly for each edition.
$env:HQ_INSTALL_NO_RUN = '0'
# A child's stderr must not abort this script under Windows PowerShell 5.1.
$ErrorActionPreference = 'Continue'
$script = Join-Path $PSScriptRoot '..\..\apps\site\public\install.ps1'
foreach ($ed in 'lite', 'full', 'auto') {
    $out = & (Get-Process -Id $PID).Path -NoProfile -File $script -Edition $ed -DryRun -Yes 2>&1 | Out-String
    Check "dry run -Edition $ed exits 0" $LASTEXITCODE 0
    Check "dry run -Edition $ed prints something" ($out.Trim().Length -gt 0) $true
}
$bad = & (Get-Process -Id $PID).Path -NoProfile -File $script -Edition nonsense 2>&1 | Out-String
Check 'a bad edition is refused' ($LASTEXITCODE -ne 0) $true

if ($script:failed -gt 0) { Write-Host "$($script:failed) check(s) failed"; exit 1 }
Write-Host 'all checks passed'
