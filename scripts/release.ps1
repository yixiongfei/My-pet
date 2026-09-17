param(
    # 跳过测试直接编译（改了几行 CSS 不想等两分钟测试的时候用）
    [switch]$SkipTests,
    # 只检查不编译：cargo test + typecheck，提交前跑一遍
    [switch]$CheckOnly
)
# 修一处 → 发布到桌面：测试 → 类型检查 → 停掉正在跑的桌宠 → 编 release exe → 用 start-vpet.ps1 拉起来。
# 平时改完代码就跑这个；它失败在哪一步就停在哪一步，不会把没过测试的版本推上桌面。
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $projectRoot 'apps\desktop'
$tauri = Join-Path $desktop 'src-tauri'

function Step($name, [scriptblock]$body) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -ne 0) { throw "$name failed (exit $LASTEXITCODE)." }
}

if (-not $SkipTests) {
    Push-Location $tauri
    try { Step 'cargo test' { cargo test --quiet } } finally { Pop-Location }
    Push-Location $projectRoot
    try { Step 'typecheck' { pnpm -r typecheck } } finally { Pop-Location }
}
if ($CheckOnly) { Write-Host 'All checks passed.' -ForegroundColor Green; return }

# 正在跑的 release exe 会占住链接器要写的文件
$running = Get-Process vpet -ErrorAction SilentlyContinue
if ($running) {
    Write-Host '==> stopping the running pet' -ForegroundColor Cyan
    $running | Stop-Process -Force
    Start-Sleep -Seconds 2
}

Push-Location $projectRoot
try { Step 'build:local (release exe, frontend embedded)' { pnpm build:local } } finally { Pop-Location }

Write-Host '==> starting' -ForegroundColor Cyan
& (Join-Path $PSScriptRoot 'start-vpet.ps1')
Write-Host 'Released to the desktop.' -ForegroundColor Green
