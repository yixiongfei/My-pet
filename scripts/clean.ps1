param(
    # 只报告会删什么，不动手
    [switch]$DryRun,
    # 连 release 的编译缓存（target\release，2–3 GB）也清掉；下次 pnpm release 要多等几分钟
    [switch]$Deep
)
# 只留正式版：清掉本机所有「不是当前 release exe」的副本和可重建的中间产物。
# 会删：debug 构建（tauri dev 用，9 GB）、前端 dist、TTS 引擎的旧 CPU 构建、探针 / 测试音频、拉模型的日志、
#       桌面上残留的 vpet*.exe 副本。
# 不删：target\release\vpet.exe（正式版）、.runtime\ollama、.runtime\models、.runtime\qwentts\build-dl、.runtime\tts-models。
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $projectRoot '.runtime'
$targets = @(
    (Join-Path $projectRoot 'apps\desktop\src-tauri\target\debug'),
    (Join-Path $projectRoot 'apps\desktop\dist'),
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
# 旧版曾把 exe 复制到桌面；桌面上的副本和仓库里的 release 同时跑会说重话
$desktop = [Environment]::GetFolderPath('Desktop')
$targets += Get-ChildItem -LiteralPath $desktop -Filter 'vpet*.exe' -ErrorAction SilentlyContinue | ForEach-Object FullName
if ($Deep) { $targets += (Join-Path $projectRoot 'apps\desktop\src-tauri\target\release') }

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
Write-Host "Kept: target\release\vpet.exe, .runtime\ollama, .runtime\models, .runtime\qwentts\build-dl, .runtime\tts-models"
