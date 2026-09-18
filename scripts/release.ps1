param(
    # 跳过测试直接编译（改了几行 CSS 不想等两分钟测试的时候用）
    [switch]$SkipTests,
    # 只检查不编译：cargo test + typecheck，提交前跑一遍
    [switch]$CheckOnly
)
# 修一处 → 发布到桌面：测试 → 类型检查 → 停掉正在跑的桌宠 → 编 release exe → 用 start-vpet.ps1 拉起来。
# 平时改完代码就跑这个；它失败在哪一步就停在哪一步，不会把没过测试的版本推上桌面。
$ErrorActionPreference = 'Stop'
$rawArgs = @($args | ForEach-Object { $_.ToString() })
$skipTestsRequested = $SkipTests.IsPresent -or $rawArgs -contains '-SkipTests'
$checkOnlyRequested = $CheckOnly.IsPresent -or $rawArgs -contains '-CheckOnly'
$projectRoot = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $projectRoot 'apps\desktop'
$tauri = Join-Path $desktop 'src-tauri'

function Step($name, [scriptblock]$body) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -ne 0) { throw "$name failed (exit $LASTEXITCODE)." }
}

if (-not $skipTestsRequested) {
    Push-Location $tauri
    try { Step 'cargo test' { cargo test --quiet } } finally { Pop-Location }
    Push-Location $projectRoot
    try { Step 'typecheck' { pnpm -r typecheck } } finally { Pop-Location }
}
if ($checkOnlyRequested) { Write-Host 'All checks passed.' -ForegroundColor Green; exit 0 }

# 正在跑的 release exe 会占住链接器要写的文件
$running = @(Get-Process -Name 'vpet' -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    Write-Host '==> stopping the running pet' -ForegroundColor Cyan
    $running | Stop-Process -Force -ErrorAction Stop
    # Windows keeps an executable locked until process teardown is complete. Waiting on the
    # process objects is deterministic; a fixed sleep occasionally raced the release linker.
    # WebView2 / 音频仍在收尾时，进程句柄偶尔会比 10 秒更晚释放；等足 30 秒仍在才算异常。
    $running | Wait-Process -Timeout 30 -ErrorAction SilentlyContinue
    $left = @(Get-Process -Name 'vpet' -ErrorAction SilentlyContinue)
    if ($left.Count -gt 0) {
        $ids = ($left | ForEach-Object Id) -join ', '
        throw "VPet did not stop; refusing to overwrite the running executable (pid $ids)."
    }
}

Push-Location $projectRoot
try { Step 'build:local (release exe, frontend embedded)' { pnpm build:local } } finally { Pop-Location }

Write-Host '==> starting' -ForegroundColor Cyan
& (Join-Path $PSScriptRoot 'start-vpet.ps1')
Write-Host 'Released to the desktop.' -ForegroundColor Green
