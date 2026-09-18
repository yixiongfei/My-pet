# 在桌面放一个「VPet 桌宠」快捷方式：点一下就跑 start-vpet.ps1（拉起 Ollama、tts-server、当前 release 的桌宠）。
# 只是个 .lnk，不复制 exe——正式版永远只有 target\release\ 那一份（CLAUDE.md）。
# 用隐藏窗口跑 PowerShell，省得每次弹一个黑框；脚本失败时弹消息框，不然什么都看不见。
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$launcher = Join-Path $projectRoot 'scripts\start-vpet.ps1'
$icon = Join-Path $projectRoot 'apps\desktop\src-tauri\icons\icon.ico'
$desktop = [Environment]::GetFolderPath('Desktop')
$lnk = Join-Path $desktop 'VPet 桌宠.lnk'

# -Command 里包一层 try/catch：start-vpet.ps1 出错会 throw，隐藏窗口下必须换成消息框
$command = "try { & '$launcher' } catch { Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show(`$_.Exception.Message, 'VPet 没起来', 'OK', 'Error') | Out-Null }"

$shell = New-Object -ComObject WScript.Shell
$s = $shell.CreateShortcut($lnk)
$s.TargetPath = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
$s.Arguments = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command `"$command`""
$s.WorkingDirectory = $projectRoot
$s.IconLocation = "$icon,0"
$s.Description = '启动 VPet 桌宠（当前 release）'
$s.WindowStyle = 7   # 最小化启动，配合 -WindowStyle Hidden 把黑框压到最短
$s.Save()
Write-Host "Shortcut written: $lnk"
