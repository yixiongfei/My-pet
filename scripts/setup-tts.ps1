param(
    [switch]$Rebuild,
    # 0.6b 在核显上约 0.75 倍实时（4 秒的话 3 秒合成），1.7b 更像真人但要慢一倍多
    [ValidateSet('0.6b', '1.7b')] [string]$Size = '0.6b',
    [ValidateSet('Q8_0', 'Q4_K_M', 'BF16')] [string]$Quant = 'Q8_0'
)
# 装好她的嗓子：编译 qwentts.cpp（Qwen3-TTS 的 GGML 移植，自带 OpenAI 兼容的 tts-server），
# 下载 CustomVoice 的 GGUF 权重。全部落在 .runtime/ 里，不碰系统目录。
# 之后 start-vpet.ps1 会自动拉起 tts-server；桌宠设置里「声音与台词」可以试听、换声音。
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDir = Join-Path $projectRoot '.runtime'
$srcDir = Join-Path $runtimeDir 'qwentts'
$buildDir = Join-Path $srcDir 'build-dl'
$modelDir = Join-Path $runtimeDir 'tts-models'
$repo = 'https://github.com/ServeurpersoCom/qwentts.cpp.git'
$hfBase = 'https://huggingface.co/Serveurperso/Qwen3-TTS-GGUF/resolve/main'
$talker = "qwen-talker-$Size-customvoice-$Quant.gguf"
$codec = "qwen-tokenizer-12hz-$Quant.gguf"
New-Item -ItemType Directory -Force -Path $runtimeDir, $modelDir | Out-Null

# 1. 源码。ggml 是子模块，一起浅克隆
if (-not (Test-Path -LiteralPath (Join-Path $srcDir 'CMakeLists.txt'))) {
    Write-Host "Cloning qwentts.cpp into $srcDir ..."
    & git clone --recurse-submodules --depth 1 --shallow-submodules $repo $srcDir
    if ($LASTEXITCODE -ne 0) { throw 'git clone failed.' }
}

# 2. 编译。用 Tauri 已经要求装好的 VS Build Tools（自带 CMake + Ninja），不另装东西。
#    动态后端（GGML_BACKEND_DL）：backend 编成单独的 DLL，运行时谁在 exe 旁边就用谁——
#    这样可以直接借 Ollama 包里的 ggml-vulkan.dll 跑核显，不用装 Vulkan SDK 重编。
$serverExe = Join-Path $buildDir 'tts-server.exe'
if ($Rebuild -or -not (Test-Path -LiteralPath $serverExe)) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio Build Tools not found (vswhere.exe missing). Install "Desktop development with C++".' }
    $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vsPath) { throw 'No Visual Studio installation with the C++ toolset was found.' }
    $vcvars = Join-Path $vsPath 'VC\Auxiliary\Build\vcvars64.bat'
    $cmakeDir = Join-Path $vsPath 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
    $ninjaDir = Join-Path $vsPath 'Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja'
    if (-not (Test-Path -LiteralPath (Join-Path $cmakeDir 'cmake.exe'))) { throw "CMake not found under $vsPath. Add the 'C++ CMake tools for Windows' component." }

    # 长句最多 300 字（Core 会按句拆），4096 位置的 KV 缓存（896 MB）用不上，砍到 2048 省一半内存
    $pipeline = Join-Path $srcDir 'src\pipeline-tts.cpp'
    $src = Get-Content -LiteralPath $pipeline -Raw
    if ($src -match 'pt->talker\.head_dim, 4096, pt->max_batch') {
        $patched = $src -replace 'pt->talker\.head_dim, 4096, pt->max_batch', 'pt->talker.head_dim, 2048, pt->max_batch'
        [IO.File]::WriteAllText($pipeline, $patched, (New-Object System.Text.UTF8Encoding $false))
        Write-Host 'Patched talker KV cache: 4096 -> 2048 positions.'
    }

    Write-Host 'Building qwentts.cpp (Release, dynamic ggml backends) ...'
    $flags = '-DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON -DGGML_BACKEND_DL=ON -DGGML_CPU=ON -DGGML_NATIVE=OFF -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON'
    $script = @"
@echo off
call "$vcvars" >nul
set PATH=$cmakeDir;$ninjaDir;%PATH%
cd /d "$srcDir"
if not exist build-dl mkdir build-dl
cd build-dl
cmake .. -G Ninja $flags
if errorlevel 1 exit /b 1
cmake --build . --config Release -j %NUMBER_OF_PROCESSORS%
"@
    $buildCmd = Join-Path $srcDir 'build-vpet.cmd'
    Set-Content -LiteralPath $buildCmd -Value $script -Encoding oem
    & cmd.exe /c $buildCmd
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $serverExe)) { throw 'qwentts.cpp build failed. See the output above.' }
}

# 3. 核显：Ollama 包里的 Vulkan 后端和这份 ggml 是同一个 ABI（版本 2），放到 exe 旁边就会被自动加载。
#    没有的话退回 CPU（0.6B 约 2 倍实时，能用但慢）。
$vulkanDll = Join-Path $runtimeDir 'ollama\lib\ollama\vulkan\ggml-vulkan.dll'
if (Test-Path -LiteralPath $vulkanDll) {
    Copy-Item -LiteralPath $vulkanDll -Destination (Join-Path $buildDir 'ggml-vulkan.dll') -Force
    Write-Host 'Vulkan backend: using ggml-vulkan.dll from the Ollama bundle (Intel/AMD iGPU acceleration).'
} else {
    Write-Warning "ggml-vulkan.dll not found at $vulkanDll; tts-server will run on the CPU."
}

# 4. 权重。0.6B Q8_0 = 969 MB talker + 291 MB tokenizer，只下一次
foreach ($file in @($talker, $codec)) {
    $dest = Join-Path $modelDir $file
    if (Test-Path -LiteralPath $dest) { continue }
    Write-Host "Downloading $file ..."
    & curl.exe -L --retry 3 -C - -o "$dest.part" "$hfBase/$file"
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $file" }
    Move-Item -LiteralPath "$dest.part" -Destination $dest -Force
}

# 5. 说一句试试。第一次要编译着色器，核显上十几秒
$probe = Join-Path $runtimeDir 'tts-probe.txt'
[IO.File]::WriteAllText($probe, '你好呀，我是萝莉斯。Hello!', (New-Object System.Text.UTF8Encoding $false))
$out = Join-Path $runtimeDir 'tts-probe.wav'
$log = Join-Path $runtimeDir 'tts-probe.log'
Write-Host 'Synthesizing a test line ...'
# PowerShell 没有 < 重定向，而且经它转手的 stdin 会按系统代码页重编码；写个 .cmd 让 cmd 直接喂文件
$probeCmd = Join-Path $runtimeDir 'tts-probe.cmd'
$exe = Join-Path $buildDir 'qwen-tts.exe'
$line = "`"$exe`" --model `"$(Join-Path $modelDir $talker)`" --codec `"$(Join-Path $modelDir $codec)`" --speaker serena --lang auto -o `"$out`" < `"$probe`" > `"$log`" 2>&1"
Set-Content -LiteralPath $probeCmd -Value "@echo off`r`n$line" -Encoding oem
& cmd.exe /c $probeCmd
$probeExit = $LASTEXITCODE
Get-Content -LiteralPath $log | Where-Object { $_ -match '^\[(Load|Perf\] Total|WAV)' } | ForEach-Object { Write-Host "  $_" }
if ($probeExit -ne 0 -or -not (Test-Path -LiteralPath $out)) { throw "Test synthesis failed, see $log" }
Write-Host "TTS is ready: $out. Run scripts\start-vpet.ps1 and she will speak."
