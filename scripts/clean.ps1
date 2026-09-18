param(
    # 只报告会删什么，不动手
    [switch]$DryRun,
    # 连当前正式版 exe / 安装包也删除；通常不要用
    [switch]$Deep
)
# 只留正式版：清掉本机所有可重建的中间产物和旧版本副本。
# 会删：debug 构建、release 链接缓存、前端 dist、旧版安装包、TTS 的旧 CPU 构建、探针 / 测试音频、
#       拉模型日志、Codex 临时日志，以及桌面上残留的 vpet*.exe 副本。
# 不删：target\release\vpet.exe（正式版）、.runtime\ollama、.runtime\models、.runtime\qwentts\build-dl、.runtime\tts-models。
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $projectRoot '.runtime'
$release = Join-Path $projectRoot 'apps\desktop\src-tauri\target\release'
$versionLine = Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') |
    Where-Object { $_ -match '^\s*"version"\s*:' } | Select-Object -First 1
if ($versionLine -notmatch '"version"\s*:\s*"([^"]+)"') { throw 'package.json version not found' }
$currentVersion = $Matches[1]
$targets = @(
    (Join-Path $projectRoot 'apps\desktop\src-tauri\target\debug'),
    (Join-Path $projectRoot 'apps\desktop\dist'),
    (Join-Path $release '.fingerprint'),
    (Join-Path $release 'build'),
    (Join-Path $release 'deps'),
    (Join-Path $release 'examples'),
    (Join-Path $release 'incremental'),
    (Join-Path $release '.cargo-artifact-lock'),
    (Join-Path $release '.cargo-build-lock'),
    (Join-Path $release '.cargo-lock'),
    (Join-Path $release 'vpet.d'),
    (Join-Path $release 'vpet.pdb'),
    (Join-Path $runtime 'qwentts\build'),
    (Join-Path $runtime 'qwentts\build-cpu.cmd'),
    (Join-Path $runtime 'qwentts\build-cpu.log'),
    (Join-Path $runtime 'qwentts\build-dl.cmd'),
    (Join-Path $runtime 'qwentts\build-dl.log'),
    (Join-Path $runtime 'tts-test'),
    (Join-Path $runtime 'tts-probe.cmd'),
    (Join-Path $runtime 'tts-probe.log'),
    (Join-Path $runtime 'tts-probe.txt'),
    (Join-Path $runtime 'tts-probe.wav'),
    (Join-Path $runtime 'ollama-windows-amd64.zip')
)
$targets += Get-ChildItem -LiteralPath $runtime -Filter 'pull-*.log' -ErrorAction SilentlyContinue | ForEach-Object FullName
$targets += Get-ChildItem -LiteralPath $projectRoot -Filter '.codex-*.log' -File -ErrorAction SilentlyContinue | ForEach-Object FullName
# GitHub 已保存每个正式版；本机 bundle 只留与 package.json 同版本的安装包和便携包。
$bundle = Join-Path $release 'bundle'
$targets += Get-ChildItem -LiteralPath $bundle -Recurse -File -Filter 'VPet_*' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notlike "VPet_${currentVersion}_*" } |
    ForEach-Object FullName
# 旧版曾把 exe 复制到桌面；桌面上的副本和仓库里的 release 同时跑会说重话
$desktop = [Environment]::GetFolderPath('Desktop')
$targets += Get-ChildItem -LiteralPath $desktop -Filter 'vpet*.exe' -ErrorAction SilentlyContinue | ForEach-Object FullName
if ($Deep) { $targets += $release }

function Size-Of($path) {
    if (Test-Path -LiteralPath $path -PathType Container) {
        (Get-ChildItem -LiteralPath $path -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
    } elseif (Test-Path -LiteralPath $path) { (Get-Item -LiteralPath $path).Length } else { 0 }
}

$running = Get-Process vpet -ErrorAction SilentlyContinue
if ($running -and -not $DryRun) {
    # 正在跑的 exe 锁着它旁边的文件；debug exe 从 tauri dev 起的也可能在跑
    $running | Where-Object { $_.Path -like '*\target\debug\*' -or $_.Path -like "$desktop*" } | ForEach-Object {
        Write-Host "Stopping non-release VPet: $($_.Path)"
        Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 1
}

$total = 0
foreach ($t in $targets) {
    if (-not (Test-Path -LiteralPath $t)) { continue }
    $bytes = Size-Of $t
    $total += $bytes
    Write-Host ("{0,8:N0} MB  {1}" -f ($bytes / 1MB), $t)
    if (-not $DryRun) { Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction SilentlyContinue }
}
Write-Host ("{0} {1:N1} GB" -f $(if ($DryRun) { 'Would free' } else { 'Freed' }), ($total / 1GB)) -ForegroundColor Green
if (-not $Deep) {
    Write-Host "Kept: target\release\vpet.exe, current-version bundle, .runtime\ollama, .runtime\models, .runtime\qwentts\build-dl, .runtime\tts-models"
}
