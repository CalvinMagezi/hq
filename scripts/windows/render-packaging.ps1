# Fills the Scoop and winget templates in packaging/ from a Lite zip and its download URL.
param([Parameter(Mandatory)][string]$Zip, [Parameter(Mandatory)][string]$Url, [string]$Out = 'packaging\out')
$ErrorActionPreference = 'Stop'
if ($Url -notmatch '^https://') { throw 'the URL must be https' }
$name = Split-Path $Zip -Leaf
if ($name -notmatch '^hq-lite-(\d+\.\d+\.\d+)-') { throw "unexpected zip name $name" }
$version = $Matches[1]
$hash = (Get-FileHash -Algorithm SHA256 -Path $Zip).Hash.ToLowerInvariant()
New-Item -ItemType Directory -Force $Out | Out-Null
foreach ($pair in @(@('packaging\scoop\hq-lite.json', 'hq-lite.json'), @('packaging\winget\hq-lite.yaml', 'hq-lite.yaml'))) {
    (Get-Content $pair[0] -Raw).Replace('@VERSION@', $version).Replace('@URL@', $Url).Replace('@SHA256@', $hash) | Set-Content (Join-Path $Out $pair[1])
}
Write-Host "Wrote manifests for $version ($hash) to $Out"
