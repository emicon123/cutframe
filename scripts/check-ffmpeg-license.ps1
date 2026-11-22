<#
.SYNOPSIS
  Fails unless a directory holds exactly the pinned FFmpeg DLLs and every one of them is an LGPL build.

.DESCRIPTION
  cargo-deny cannot see FFmpeg, so this is the licence gate for the DLLs (CLAUDE.md rule 7: FFmpeg is
  LGPL only, dynamically linked, no GPL components). It reads the DLL files directly, so it needs no
  ffmpeg.exe and works on an installed copy of the app as well as on the provisioned `.ffmpeg\bin`.

  Each FFmpeg library embeds two strings that this script checks in every DLL it finds:
    - "lib<name> license: <licence>"  (what avutil_license() etc. return): must start with "LGPL"
    - the `configure` command line    (what avutil_configuration() returns): must not contain
      --enable-gpl, --enable-nonfree or a GPL-only encoder library (libx264, libx265, libxvid, ...)
  It also fails if a GPL-only library DLL (postproc) or an x264/x265/xvid/fdk-aac DLL is present, and
  if any DLL listed in scripts/ffmpeg.json is missing.

  Used by scripts/fetch-ffmpeg.ps1 (local + CI provisioning) and by the CI smoke test on the
  installed app. Compatible with Windows PowerShell 5.1 and PowerShell 7.

.PARAMETER Path
  Directory that holds the FFmpeg DLLs (for example .ffmpeg\bin or %LOCALAPPDATA%\Cutframe).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check-ffmpeg-license.ps1 -Path .ffmpeg\bin
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string] $Path
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$pin = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'ffmpeg.json') | ConvertFrom-Json
if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw "FFmpeg DLL directory not found: $Path" }

$problems = New-Object System.Collections.Generic.List[string]

# 1. Every pinned DLL is present (the loader needs all of them next to the .exe).
foreach ($name in $pin.dlls) {
  if (-not (Test-Path -LiteralPath (Join-Path $Path $name) -PathType Leaf)) { $problems.Add("missing $name") }
}

# 2. No GPL-only or nonfree library DLLs at all.
$forbiddenDlls = 'postproc-*.dll', '*x264*.dll', '*x265*.dll', '*xvid*.dll', '*fdk-aac*.dll', '*fdk_aac*.dll'
foreach ($pattern in $forbiddenDlls) {
  foreach ($file in @(Get-ChildItem -LiteralPath $Path -Filter $pattern -File)) {
    $problems.Add("forbidden (GPL-only or nonfree) DLL present: $($file.Name)")
  }
}

# 3. Every FFmpeg library DLL reports an LGPL licence and an LGPL configure line.
$latin1 = [System.Text.Encoding]::GetEncoding(28591) # byte-to-char 1:1, so binary data is safe to search
$forbiddenFlags = '--enable-gpl', '--enable-nonfree', '--enable-libx264', '--enable-libx265',
  '--enable-libxvid', '--enable-libfdk-aac', '--enable-libvidstab', '--enable-librubberband',
  '--enable-frei0r', '--enable-libdavs2', '--enable-libxavs2', '--enable-avisynth'
$libs = @(Get-ChildItem -LiteralPath $Path -File | Where-Object { $_.Name -match '^(av|sw|postproc)[a-z]*-\d+\.dll$' })
if ($libs.Count -eq 0) { $problems.Add("no FFmpeg library DLLs found in $Path") }

foreach ($dll in $libs) {
  $text = $latin1.GetString([System.IO.File]::ReadAllBytes($dll.FullName))

  $licences = @([regex]::Matches($text, 'lib[a-z]+ license: ([ -~]+)') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)
  if ($licences.Count -eq 0) {
    $problems.Add("$($dll.Name): no embedded licence string found")
  }
  foreach ($licence in $licences) {
    if ($licence -notmatch '^LGPL') { $problems.Add("$($dll.Name): licence is '$licence', expected LGPL") }
  }

  $configure = [regex]::Match($text, '--prefix=[ -~]+')
  if (-not $configure.Success) {
    $problems.Add("$($dll.Name): no embedded configure line found")
  } else {
    $flags = $configure.Value -split ' '
    foreach ($flag in $forbiddenFlags) {
      if ($flags -contains $flag) { $problems.Add("$($dll.Name): configured with $flag") }
    }
  }

  $shown = if ($licences.Count -gt 0) { $licences -join ', ' } else { '?' }
  Write-Host ("  {0,-20} {1}" -f $dll.Name, $shown)
}

if ($problems.Count -gt 0) {
  foreach ($p in $problems) {
    if ($env:GITHUB_ACTIONS -eq 'true') { Write-Host "::error title=FFmpeg licence check::$p" } else { Write-Host "ERROR: $p" -ForegroundColor Red }
  }
  Write-Host "FFmpeg licence check FAILED for $Path ($($problems.Count) problem(s)). Only the pinned LGPL shared build is allowed (CLAUDE.md rule 7)." -ForegroundColor Red
  exit 1
}
Write-Host "FFmpeg licence check passed: $($libs.Count) LGPL DLLs in $Path"
exit 0
