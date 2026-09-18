# VPet · 项目约定

有身体的 Personal Agent：Tauri 2 + React（Body）+ Rust（Core，确定性）+ 本机 Ollama / Qwen3-TTS。
方案在 `docs/`（索引 `docs/README.md`），进度在 `docs/06-roadmap.md`，变更在 `CHANGELOG.md`。

## 改代码的同一次提交里必须同步文档

- 加 / 改功能 → `CHANGELOG.md`「未发布」段加一条（写**为什么这么做**，不只是做了什么）。
- 进度变化 → `docs/06-roadmap.md` 对应行的 ✅ / 🚧 / ⬜。
- 模块 / 事件 / 目录变了 → `docs/02-components.md`。
- 数值 / 阈值 / 决策 / 健康 → `docs/03-core.md`。
- 布局 / 动画 / 气泡 / 交互 → `docs/04-body-animation.md`。
- 对话 / 意图 / 记忆 / 语音 / 台词 → `docs/05-brain.md`。
- 用户能感知的行为变了 → `README.md`「和她相处」。

## 工作流

- 提交前：`pnpm check`（cargo test + typecheck）。
- 发布到桌面：`pnpm release`（停桌宠 → 编 release exe → `start-vpet.ps1` 拉起 Ollama、tts-server、桌宠）。正在跑的 release exe 会占住链接器，脚本会先停它。
- 发正式版到 GitHub：`pnpm release:github -- -Version X.Y.Z`（要求工作区干净、gh 已登录；会改版本号、切 CHANGELOG、打 tag、建 Release）。本机只留正式版：`pnpm clean`。
- 只有一个远程 `origin`（github.com/yixiongfei/My-pet）；正式版 exe 只在 `apps/desktop/src-tauri/target/release/`，别复制到桌面。
- 语音引擎 / 模型权重 / Ollama 都在 `.runtime/`（不入库）；`scripts/setup-tts.ps1` 只需跑一次。
- 角色美术 `assets-src/` 归原作者，不入库；`legacy/` 是原版 C# 只读参考，不入库。

## 代码约定

- 状态机 `reduce` 保持纯函数：时间、随机数从外面喂；新行为先写单元测试。
- 用户意志进状态机只走 `request_action`（服从判定），对话里的使唤也不例外。
- Body 不做业务逻辑：数值、台词、动作由 Core 事件推过来。
- 只接本机地址（`chat::local_endpoint`），不联网、不上传。
- 注释和文档用中文，写清「为什么」；新常量放在文件顶部并注明单位。
- 前后端各有一份的常量（`HEAD_ROOM`、`SICK_HEALTH` / `ILL_HEALTH`、`SPEAKERS`、默认设置）改要成对改。
