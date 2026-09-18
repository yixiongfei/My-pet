param(
    # 版本号，如 0.1.0。不给就把补丁号 +1（0.0.1 → 0.0.2）
    [string]$Version,
    # 跳过 cargo test + typecheck（不建议）
    [switch]$SkipChecks,
    # 只打包、打 tag、推送，不建 GitHub Release（比如 gh 没登录）
    [switch]$NoUpload,
    # 试跑：不改文件、不提交、不推送、不上传，只做检查和打包
    [switch]$DryRun,
    [string]$Remote = 'origin'
)
# 发一个正式版到 GitHub Releases：
#   检查工作区干净 → cargo test + typecheck → 版本号写进 4 处 → CHANGELOG「未发布」改成本版
#   → tauri build（NSIS 安装包 + 便携 zip）→ commit「release: vX.Y.Z」+ tag → push
#   → gh release create（附上安装包 / zip，说明取自 CHANGELOG 这一节）→ 重新拉起桌宠。
# 前提：gh 已登录（gh auth login）；仓库里没有未提交的改动。
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$tauriDir = Join-Path $projectRoot 'apps\desktop\src-tauri'
$conf = Join-Path $tauriDir 'tauri.conf.json'
Set-Location $projectRoot

function Step($name, [scriptblock]$body) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -ne 0) { throw "$name failed (exit $LASTEXITCODE)." }
}

# 0. 前提
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
if ($branch -ne 'main') { throw "Release from 'main' only (current: $branch)." }
if (-not $DryRun -and (git status --porcelain)) { throw 'Working tree is not clean. Commit or stash first.' }
if (-not $NoUpload) {
    & gh auth status | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'gh is not logged in. Run `gh auth login` once, or pass -NoUpload to skip the GitHub Release.' }
}

# 1. 版本号
$current = ([regex]::Match((Get-Content -LiteralPath $conf -Raw), '"version"\s*:\s*"([^"]+)"')).Groups[1].Value
if (-not $Version) {
    $parts = $current.Split('.')
    $parts[-1] = [string]([int]$parts[-1] + 1)
    $Version = $parts -join '.'
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3 (got $Version)." }
if ((git tag -l "v$Version")) { throw "Tag v$Version already exists." }
$date = Get-Date -Format 'yyyy-MM-dd'
Write-Host "Releasing v$Version (was $current)" -ForegroundColor Green

# 2. 检查
if (-not $SkipChecks) {
    Push-Location $tauriDir
    try { Step 'cargo test' { cargo test --quiet } } finally { Pop-Location }
    Step 'typecheck' { pnpm -r typecheck }
}

# 3. 版本号写进四处 + CHANGELOG
$utf8 = New-Object System.Text.UTF8Encoding $false
function Bump($path, $pattern, $replacement) {
    $text = [IO.File]::ReadAllText($path)
    $new = [regex]::Replace($text, $pattern, $replacement, 1)
    if ($new -eq $text) { throw "Version field not found in $path" }
    if (-not $DryRun) { [IO.File]::WriteAllText($path, $new, $utf8) }
}
Bump (Join-Path $projectRoot 'package.json') '("version"\s*:\s*")[^"]+(")' "`${1}$Version`${2}"
Bump (Join-Path $projectRoot 'apps\desktop\package.json') '("version"\s*:\s*")[^"]+(")' "`${1}$Version`${2}"
Bump $conf '("version"\s*:\s*")[^"]+(")' "`${1}$Version`${2}"
Bump (Join-Path $tauriDir 'Cargo.toml') '(?m)^(version\s*=\s*")[^"]+(")' "`${1}$Version`${2}"

$changelogPath = Join-Path $projectRoot 'CHANGELOG.md'
$changelog = [IO.File]::ReadAllText($changelogPath)
$m = [regex]::Match($changelog, '(?m)^## 未发布[^\r\n]*\r?\n')
if (-not $m.Success) { throw 'CHANGELOG.md has no "## 未发布" section.' }
$sectionStart = $m.Index + $m.Length
$next = [regex]::Match($changelog.Substring($sectionStart), '(?m)^## ')
$sectionEnd = if ($next.Success) { $sectionStart + $next.Index } else { $changelog.Length }
$notes = $changelog.Substring($sectionStart, $sectionEnd - $sectionStart).Trim()
if (-not $notes) { throw 'The "## 未发布" section is empty — nothing to release.' }
$head = "## 未发布`r`n`r`n### 新增`r`n`r`n### 修复`r`n`r`n## v$Version · $date`r`n"
$newChangelog = $changelog.Substring(0, $m.Index) + $head + $changelog.Substring($sectionStart)
if (-not $DryRun) { [IO.File]::WriteAllText($changelogPath, $newChangelog, $utf8) }
$notesFile = Join-Path $env:TEMP "vpet-release-notes-$Version.md"
[IO.File]::WriteAllText($notesFile, $notes + "`r`n`r`n---`r`n角色美术来自 VPet-Simulator（LorisYounger），归原作者所有；本安装包仅供个人使用。`r`n", $utf8)

# 4. 打包。正在跑的 release exe 会锁住链接器要写的文件
$running = @(Get-Process -Name vpet -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    Write-Host '==> stopping the running pet' -ForegroundColor Cyan
    $running | Stop-Process -Force
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}
Step 'tauri build (NSIS installer)' { pnpm build }
$installer = Get-ChildItem -LiteralPath (Join-Path $tauriDir 'target\release\bundle\nsis') -Filter '*-setup.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) { throw 'Installer not found under target\release\bundle\nsis.' }
$portable = Join-Path $tauriDir "target\release\bundle\VPet_${Version}_x64-portable.zip"
if (Test-Path -LiteralPath $portable) { Remove-Item -LiteralPath $portable -Force }
Compress-Archive -LiteralPath (Join-Path $tauriDir 'target\release\vpet.exe'), (Join-Path $projectRoot '启动桌宠.cmd'), (Join-Path $projectRoot 'scripts\start-vpet.ps1'), (Join-Path $projectRoot 'scripts\setup-tts.ps1') -DestinationPath $portable
Write-Host "  $($installer.FullName)"
Write-Host "  $portable"

if ($DryRun) { Write-Host "Dry run: built v$Version, nothing committed or uploaded." -ForegroundColor Yellow; exit 0 }

# 5. 提交 + tag + 推送
Step 'git commit' { git add package.json apps/desktop/package.json $conf (Join-Path $tauriDir 'Cargo.toml') (Join-Path $tauriDir 'Cargo.lock') CHANGELOG.md; git commit -q -m "release: v$Version" }
Step 'git tag' { git tag -a "v$Version" -m "VPet v$Version" }
Step 'git push' { git push $Remote main "v$Version" }

# 6. GitHub Release
if (-not $NoUpload) {
    Step 'gh release create' { gh release create "v$Version" $installer.FullName $portable --title "VPet v$Version" --notes-file $notesFile }
    $url = (gh release view "v$Version" --json url --jq .url)
    Write-Host "Released: $url" -ForegroundColor Green
} else {
    Write-Host "Tag v$Version pushed. Upload later with: gh release create v$Version `"$($installer.FullName)`" `"$portable`" --notes-file `"$notesFile`"" -ForegroundColor Yellow
}

# 7. 桌面上换成新版
& (Join-Path $PSScriptRoot 'start-vpet.ps1')
