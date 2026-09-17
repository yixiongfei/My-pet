param([switch]$Rebuild)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDir = Join-Path $projectRoot '.runtime'
$ollamaExe = Join-Path $runtimeDir 'ollama\ollama.exe'
# Release build with the frontend embedded. target\debug\vpet.exe is the `cargo run` / `tauri dev`
# binary: it expects the Vite dev server on :1420 and must not be used here.
$petExe = Join-Path $projectRoot 'apps\desktop\src-tauri\target\release\vpet.exe'
# Chat model + embedding model (semantic memory retrieval). Both are pulled once if missing.
$chatModel = 'qwen3.5:9b'
$embedModel = 'qwen3-embedding:0.6b'
# Her voice: qwentts.cpp tts-server + Qwen3-TTS CustomVoice GGUFs, installed by scripts\setup-tts.ps1.
# Optional: when anything is missing the pet runs mute (bubbles only) and the settings page says why.
$ttsExe = Join-Path $runtimeDir 'qwentts\build-dl\tts-server.exe'
$ttsModelDir = Join-Path $runtimeDir 'tts-models'
$ttsPort = 8090
New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null

# Process-local settings only; keep model weights in this project on D:.
$env:OLLAMA_HOST = '127.0.0.1:11434'
$env:OLLAMA_MODELS = Join-Path $runtimeDir 'models'
# Run inference on the GPU. Ollama drops integrated GPUs by default; the Intel Arc iGPU
# via Vulkan cuts the 9B model's first-token latency from ~4.8 s to ~1.9 s and keeps the CPU free.
$env:OLLAMA_VULKAN = '1'
$env:OLLAMA_IGPU_ENABLE = '1'
$env:OLLAMA_KEEP_ALIVE = '30m'

# Orphaned Ollama runners: when ollama.exe dies (crash, killed, machine sleep) its llama-server children can
# outlive it and keep several GB of commit charge each. Two of them once pushed this PC to the page-file limit.
Get-CimInstance Win32_Process -Filter "Name='llama-server.exe'" | ForEach-Object {
    if (-not (Get-Process -Id $_.ParentProcessId -ErrorAction SilentlyContinue)) {
        Write-Host "Killing orphaned llama-server (pid $($_.ProcessId))"
        Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

$ollamaReady = $false
try { $null = Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 2; $ollamaReady = $true } catch {}
if (-not $ollamaReady -and (Test-Path -LiteralPath $ollamaExe)) {
    Start-Process -FilePath $ollamaExe -ArgumentList 'serve' -WorkingDirectory $runtimeDir -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $runtimeDir 'ollama-server.log') `
        -RedirectStandardError (Join-Path $runtimeDir 'ollama-server-error.log') | Out-Null
    foreach ($i in 1..20) {
        Start-Sleep -Milliseconds 500
        try { $null = Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 2; $ollamaReady = $true; break } catch {}
    }
}

# Pull anything missing. This is the one place downloads happen (6.6 GB + 0.6 GB on first run).
if ($ollamaReady -and (Test-Path -LiteralPath $ollamaExe)) {
    try {
        $installed = (Invoke-RestMethod 'http://127.0.0.1:11434/api/tags' -TimeoutSec 5).models | ForEach-Object { $_.name }
        foreach ($model in @($chatModel, $embedModel)) {
            if ($installed -notcontains $model -and $installed -notcontains "${model}:latest") {
                Write-Host "Downloading $model (first run only)..."
                & $ollamaExe pull $model
            }
        }
    } catch { Write-Warning "Could not check installed models: $_" }
}

# tts-server: listens on 127.0.0.1 only, picks the Vulkan backend when ggml-vulkan.dll sits next to it.
$ttsReady = $false
try { $null = Invoke-RestMethod "http://127.0.0.1:$ttsPort/health" -TimeoutSec 2; $ttsReady = $true } catch {}
if (-not $ttsReady) {
    $talker = Get-ChildItem -LiteralPath $ttsModelDir -Filter 'qwen-talker-*-customvoice-*.gguf' -ErrorAction SilentlyContinue | Select-Object -First 1
    $codec = Get-ChildItem -LiteralPath $ttsModelDir -Filter 'qwen-tokenizer-12hz-*.gguf' -ErrorAction SilentlyContinue | Select-Object -First 1
    if ((Test-Path -LiteralPath $ttsExe) -and $talker -and $codec) {
        Start-Process -FilePath $ttsExe -WorkingDirectory (Split-Path -Parent $ttsExe) -WindowStyle Hidden `
            -ArgumentList @('--model', "`"$($talker.FullName)`"", '--codec', "`"$($codec.FullName)`"", '--alias', 'qwen3-tts', '--host', '127.0.0.1', '--port', "$ttsPort") `
            -RedirectStandardOutput (Join-Path $runtimeDir 'tts-server.log') `
            -RedirectStandardError (Join-Path $runtimeDir 'tts-server-error.log') | Out-Null
        Write-Host 'Starting tts-server (her voice) ...'
    } else {
        Write-Host 'Voice not installed; run scripts\setup-tts.ps1 once if you want her to speak.'
    }
}

$allPets = @(Get-Process vpet -ErrorAction SilentlyContinue)
$runningPet = @($allPets | Where-Object { $_.Path -eq $petExe })
if ($runningPet.Count -gt 0) {
    # 旧版曾把 exe 复制到桌面；只按路径判断会让桌面副本和仓库 release 同时运行，
    # 整点动作台词就会被两份状态机重复播放。保留一份当前 release，其余全部停掉。
    $keep = $runningPet | Select-Object -First 1
    $allPets | Where-Object { $_.Id -ne $keep.Id } | Stop-Process -Force -ErrorAction SilentlyContinue
    Write-Host 'VPet is already running. Click the pet or press Alt+V to chat.'
    return
}
if ($allPets.Count -gt 0) {
    Write-Host 'Stopping stale VPet copies before starting the current release...'
    $allPets | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
}
if ($Rebuild -or -not (Test-Path -LiteralPath $petExe)) {
    Push-Location $projectRoot
    try {
        & pnpm build:local
        if ($LASTEXITCODE -ne 0) { throw 'VPet build failed.' }
    } finally { Pop-Location }
}
# build:local (tauri build --no-bundle) embeds the frontend: no terminal/dev server needs to stay open.
Start-Process -FilePath $petExe -WorkingDirectory $projectRoot -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $runtimeDir 'vpet.log') `
    -RedirectStandardError (Join-Path $runtimeDir 'vpet-error.log') | Out-Null
Write-Host 'VPet started. Click the pet to chat; right-click for settings.'
