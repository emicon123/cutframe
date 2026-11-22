<#
.SYNOPSIS
  Downloads the pinned LGPL shared FFmpeg build for Windows x64 into .ffmpeg\, verifies its SHA-256
  and licence, and sets the environment the Rust build (rsmpeg / rusty_ffmpeg) needs.

.DESCRIPTION
  The pin (URL, version, SHA-256, DLL list) lives in scripts/ffmpeg.json, which CI reads through this
  same script. Run it from the repository root, locally with `pnpm setup:ffmpeg`, or directly:

      powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fetch-ffmpeg.ps1

  Idempotent: when .ffmpeg\ already holds the pinned build (marker file .ffmpeg\.sha256), nothing is
  downloaded; the licence check and the environment setup still run. -Force re-downloads.
  The script refuses to extract an archive whose SHA-256 differs from the pin, and deletes .ffmpeg\
  again if the licence check (scripts/check-ffmpeg-license.ps1) fails. Compatible with Windows
  PowerShell 5.1 and PowerShell 7. Needs no admin rights.

  Layout after a successful run (the folder is gitignored):
      .ffmpeg\bin\       avcodec-61.dll ... swscale-8.dll (+ ffmpeg.exe, ffprobe.exe for tests/tools)
      .ffmpeg\lib\       MSVC import libraries avcodec.lib ... swscale.lib (+ .def, .dll.a, pkgconfig)
      .ffmpeg\include\   libavcodec\ ... libswscale\ headers
      .ffmpeg\LICENSE.txt  FFmpeg's LGPL text      .ffmpeg\NOTICE.txt  version, source and configure line
  src-tauri/tauri.conf.json bundles bin\*.dll next to Cutframe.exe and the two text files under
  licenses\ffmpeg\ (NSIS install dir; tauri-build also copies them next to the dev exe).

  ENVIRONMENT CONTRACT (what the Rust build and tests rely on)
    FFMPEG_DIR          <repo>\.ffmpeg                 root of the provisioned build
    FFMPEG_INCLUDE_DIR  <repo>\.ffmpeg\include         rusty_ffmpeg: headers for bindgen
    FFMPEG_LIBS_DIR     <repo>\.ffmpeg\lib             rusty_ffmpeg: link search path (import .lib files)
    FFMPEG_LINK_MODE    dynamic                        rusty_ffmpeg: link-lib=dylib=avcodec... against the
                                                       import libraries, so the DLLs load at run time
    LIBCLANG_PATH       C:\Program Files\LLVM\bin      bindgen; only set if unset and libclang.dll is there
    PATH                += <repo>\.ffmpeg\bin          so `cargo test` binaries find the DLLs
  FFMPEG_DLL_PATH and FFMPEG_PKG_CONFIG_PATH are deliberately NOT set: on Windows rusty_ffmpeg ignores
  pkg-config, and FFMPEG_DLL_PATH expects the .lib files next to the .dll files, which this build
  does not have. Nothing else needs to be configured for `cargo build` / `cargo test`.

  Where the variables go:
    - always: the current PowerShell process
    - in GitHub Actions ($env:GITHUB_ACTIONS): $env:GITHUB_ENV and $env:GITHUB_PATH for later steps
    - locally: persisted as user environment variables (HKCU\Environment), unless -NoUserEnv.
      Open a new terminal afterwards. With several checkouts, the last one set up wins.

  PINNING NOTE: the archive is a dated BtbN/FFmpeg-Builds month-end autobuild. BtbN prunes old
  autobuilds (month-end builds are kept two years). If the URL stops working, mirror the same zip
  (same SHA-256) to a release of this repository and change "url" in scripts/ffmpeg.json.

.PARAMETER Dir
  Target folder. Default: .ffmpeg in the repository root.
.PARAMETER Force
  Re-download and re-extract even if the pinned build is already present.
.PARAMETER NoUserEnv
  Do not persist user environment variables (the current process and GitHub Actions still get them).
#>
[CmdletBinding()]
param(
  [string] $Dir,
  [switch] $Force,
  [switch] $NoUserEnv
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 leaves $PSScriptRoot empty inside param() defaults, so the default is set here.
if (-not $Dir) { $Dir = Join-Path (Split-Path -Parent $PSScriptRoot) '.ffmpeg' }
$ProgressPreference = 'SilentlyContinue' # Invoke-WebRequest is very slow with the progress bar in 5.1

$pin = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'ffmpeg.json') | ConvertFrom-Json
$Dir = [System.IO.Path]::GetFullPath($Dir)
$marker = Join-Path $Dir '.sha256'
$bin = Join-Path $Dir 'bin'
$isCi = $env:GITHUB_ACTIONS -eq 'true'

function Test-Provisioned {
  if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) { return $false }
  if ((Get-Content -Raw -LiteralPath $marker).Trim() -ne $pin.sha256) { return $false }
  foreach ($name in $pin.dlls) {
    if (-not (Test-Path -LiteralPath (Join-Path $bin $name) -PathType Leaf)) { return $false }
  }
  foreach ($sub in 'lib\avutil.lib', 'include\libavutil\avutil.h', 'LICENSE.txt') {
    if (-not (Test-Path -LiteralPath (Join-Path $Dir $sub) -PathType Leaf)) { return $false }
  }
  return $true
}

function Get-ConfigureLine([string] $dll) {
  $text = [System.Text.Encoding]::GetEncoding(28591).GetString([System.IO.File]::ReadAllBytes($dll))
  $m = [regex]::Match($text, '--prefix=[ -~]+')
  if ($m.Success) { return $m.Value } else { return '(not found)' }
}

function Install-Pinned {
  $tempRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
  $zip = Join-Path $tempRoot $pin.archive
  $staging = "$Dir.tmp"

  # A cached zip with the right hash is reused; anything else is downloaded again.
  $haveZip = (Test-Path -LiteralPath $zip -PathType Leaf) -and
    ((Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash -eq $pin.sha256.ToUpperInvariant())
  if (-not $haveZip) {
    Write-Host "Downloading $($pin.url)"
    if ($PSVersionTable.PSVersion.Major -lt 6) {
      [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }
    Invoke-WebRequest -UseBasicParsing -Uri $pin.url -OutFile $zip
  }

  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
  if ($actual -ne $pin.sha256.ToLowerInvariant()) {
    Remove-Item -LiteralPath $zip -Force
    throw "SHA-256 mismatch for $($pin.archive): expected $($pin.sha256), got $actual. Refusing to use it."
  }
  Write-Host "SHA-256 verified: $actual"

  if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  [System.IO.Compression.ZipFile]::ExtractToDirectory($zip, $staging)

  # The zip holds one top-level folder named after the archive.
  $top = @(Get-ChildItem -LiteralPath $staging -Directory)
  if ($top.Count -ne 1) { throw "Unexpected archive layout: $($top.Count) top-level folders in $($pin.archive)" }
  $src = $top[0].FullName

  if (Test-Path -LiteralPath $Dir) { Remove-Item -LiteralPath $Dir -Recurse -Force }
  New-Item -ItemType Directory -Path $Dir | Out-Null
  foreach ($item in 'bin', 'lib', 'include', 'LICENSE.txt') {
    $from = Join-Path $src $item
    if (-not (Test-Path -LiteralPath $from)) { throw "Archive is missing $item" }
    Move-Item -LiteralPath $from -Destination (Join-Path $Dir $item)
  }
  Remove-Item -LiteralPath $staging -Recurse -Force
  Remove-Item -LiteralPath $zip -Force

  # Notice shipped next to the LGPL text in the installer (LGPL obligations: say which library,
  # under which licence, where its exact source is, and that the user may replace the DLLs).
  $configure = Get-ConfigureLine (Join-Path $bin 'avutil-59.dll')
  $notice = @"
FFmpeg libraries used by Cutframe
=================================

Cutframe uses the FFmpeg libraries (libavcodec, libavdevice, libavfilter, libavformat, libavutil,
libswresample, libswscale), version $($pin.version) (release branch $($pin.branch)), licensed under the
GNU Lesser General Public License version 3 or later (LGPL-3.0-or-later). The full licence text is in
LICENSE.txt next to this file. FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg
project. Cutframe is not affiliated with the FFmpeg project.

The libraries are unmodified builds, dynamically linked, and installed as separate DLL files next to
Cutframe.exe ($(($pin.dlls) -join ', ')).
You may replace them with your own build of the same FFmpeg release branch.

Corresponding source code
  FFmpeg source:        $($pin.sourceCode)
  Build scripts:        $($pin.buildScripts)
  Binary archive:       $($pin.url)
  Archive SHA-256:      $($pin.sha256)

Build configuration (no --enable-gpl, no --enable-nonfree)
  $configure
"@
  Set-Content -LiteralPath (Join-Path $Dir 'NOTICE.txt') -Value $notice -Encoding Ascii
}

# --- provision ------------------------------------------------------------------------------------
if ($Force -or -not (Test-Provisioned)) {
  Install-Pinned
} else {
  Write-Host "FFmpeg $($pin.version) already provisioned in $Dir"
}

# --- licence gate (runs every time, also on an existing folder) -----------------------------------
& (Join-Path $PSScriptRoot 'check-ffmpeg-license.ps1') -Path $bin
if ($LASTEXITCODE -ne 0) {
  Remove-Item -LiteralPath $Dir -Recurse -Force
  throw "The FFmpeg build in scripts/ffmpeg.json failed the licence check; $Dir was removed."
}
# The marker is written only after the hash and the licence check passed.
Set-Content -LiteralPath $marker -Value $pin.sha256 -Encoding Ascii -NoNewline

# --- environment ----------------------------------------------------------------------------------
$vars = [ordered]@{
  FFMPEG_DIR         = $Dir
  FFMPEG_INCLUDE_DIR = (Join-Path $Dir 'include')
  FFMPEG_LIBS_DIR    = (Join-Path $Dir 'lib')
  FFMPEG_LINK_MODE   = 'dynamic'
}
if (-not $env:LIBCLANG_PATH) {
  $llvm = Join-Path $env:ProgramFiles 'LLVM\bin'
  if (Test-Path -LiteralPath (Join-Path $llvm 'libclang.dll')) { $vars['LIBCLANG_PATH'] = $llvm }
  else { Write-Warning 'LIBCLANG_PATH is not set and LLVM was not found in Program Files; bindgen (rsmpeg) needs libclang. Install it with: winget install -e --id LLVM.LLVM --version 20.1.8' }
}
# bindgen 0.71 (pulled in by rusty_ffmpeg 0.16 / FFmpeg 7.1) emits forward-declared structs such as
# AVFormatContext as opaque types with libclang 21 or newer, and rsmpeg then fails to compile.
$libclangDir = if ($vars.Contains('LIBCLANG_PATH')) { $vars['LIBCLANG_PATH'] } else { $env:LIBCLANG_PATH }
if ($libclangDir) {
  $libclang = Join-Path $libclangDir 'libclang.dll'
  if (Test-Path -LiteralPath $libclang -PathType Leaf) {
    $major = (Get-Item -LiteralPath $libclang).VersionInfo.FileMajorPart
    if ($major -ge 21) {
      Write-Warning "libclang $major found in $libclangDir; the FFmpeg bindings need LLVM 20 or older (bindgen 0.71). Install it with: winget install -e --id LLVM.LLVM --version 20.1.8"
    }
  }
}

foreach ($name in $vars.Keys) { Set-Item -Path "env:$name" -Value $vars[$name] }
if (($env:Path -split ';') -notcontains $bin) { $env:Path = "$bin;$env:Path" }

if ($isCi) {
  foreach ($name in $vars.Keys) { Add-Content -LiteralPath $env:GITHUB_ENV -Value "$name=$($vars[$name])" -Encoding utf8 }
  Add-Content -LiteralPath $env:GITHUB_PATH -Value $bin -Encoding utf8
} elseif (-not $NoUserEnv) {
  # PATH is edited in the registry so %VAR% references in the user PATH stay unexpanded.
  $key = Get-Item -LiteralPath 'HKCU:\Environment'
  $userPath = [string] $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
  $entries = @($userPath -split ';' | Where-Object { $_ -and ($_ -notlike '*\.ffmpeg\bin') })
  $newPath = (@($bin) + $entries) -join ';'
  if ($newPath -ne $userPath) {
    Set-ItemProperty -LiteralPath 'HKCU:\Environment' -Name 'Path' -Value $newPath -Type ExpandString
  }
  # SetEnvironmentVariable also broadcasts WM_SETTINGCHANGE, so new terminals see the PATH change too.
  foreach ($name in $vars.Keys) { [Environment]::SetEnvironmentVariable($name, $vars[$name], 'User') }
  Write-Host 'User environment updated. Open a new terminal (or restart your editor) to pick it up.'
}

Write-Host ''
Write-Host "FFmpeg $($pin.version) ($($pin.variant)) ready in $Dir"
foreach ($name in $vars.Keys) { Write-Host ("  {0,-19}= {1}" -f $name, $vars[$name]) }
Write-Host "  PATH              += $bin"
