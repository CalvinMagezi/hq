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
    (Rel 'v3' $true @($zip, "$zip.sha256")),
    (Rel 'v2' $false @('hq-0.9.1-linux-x86_64.tar.gz')),
    (Rel 'v1' $false @($zip)),
    (Rel 'v0' $false @($zip, "$zip.sha256"))
)
$pick = Select-HqLiteAsset -Releases $rels
Check 'skips drafts, releases without a lite zip, and zips without a checksum' $pick.Tag 'v0'
Check 'the asset urls are carried' $pick.SumUrl "https://example.test/v0/$zip.sha256"
Check 'a named tag is honoured' (Select-HqLiteAsset -Releases $rels -Tag 'v1') $null
Check 'nothing found -> null' (Select-HqLiteAsset -Releases @()) $null

# Checksum file parsing, as written by the Windows Lite workflow (LF) and by sha256sum (CRLF-tolerant).
$h = ('ab' * 32)
Check 'hash is read'            (Get-HqExpectedHash -Text "$h  $zip`n" -FileName $zip) $h
Check 'hash is lowercased'      (Get-HqExpectedHash -Text ("AB" * 32 + "  $zip`r`n") -FileName $zip) $h
Check 'binary-mode star'        (Get-HqExpectedHash -Text "$h *$zip`n" -FileName $zip) $h
Check 'wrong file name -> null' (Get-HqExpectedHash -Text "$h  other.zip`n" -FileName $zip) $null
Check 'garbage -> null'         (Get-HqExpectedHash -Text "not a checksum" -FileName $zip) $null

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
