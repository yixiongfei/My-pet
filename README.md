# VPet · Personal Agent

> 一个有身体、有状态、有记忆、能看懂你的工作/学习环境，并通过自然语言陪你工作和学习的 Personal Agent。
> 身体来自 [VPet-Simulator](https://github.com/LorisYounger/VPet) 的动画资产；大脑是新的。

**当前阶段：Body / Core 已可日常使用，Agent 与知识库能力继续按路线图补齐。** 完整实现状态见 [docs/](docs/README.md) 与 [路线图](docs/07-roadmap.md)。

## 目录

```
docs/            技术方案（01 产品定义 … 08 待确认问题）
assets-src/      原版动画帧与 vup.lps —— 美术归原作者，不进仓库（见 assets-src/README.md）
scripts/         build-assets.mjs（PNG → WebP + manifest）· start-vpet.ps1（一键启动）· setup-tts.ps1 · release.ps1
packages/shared  TS/Rust 共用的 JSON 契约（zod）
packages/brain   Agent 编排（纯 TS，无 UI）—— Phase 3 起
apps/desktop     Tauri 2 + React：Body（前端）与 Core（src-tauri，Rust）
training/        用你点赞 / 修订过的回答做 LoRA 的脚本
.runtime/        本机运行时（Ollama、TTS 引擎、模型权重），脚本自动生成，不进仓库
```

## 跑起来

前置：Node ≥ 22、pnpm、Rust stable（`rustup`）、Visual Studio C++ Build Tools、WebView2（Win11 自带）。

```bash
pnpm install
# 把原版 VPet 的美术放进 assets-src/（结构见 assets-src/README.md）
pnpm build:assets      # 首次约 3–5 分钟，生成 apps/desktop/public/pet/
pnpm dev               # tauri dev：桌面上出现宠物
pnpm build             # 产出 apps/desktop/src-tauri/target/release/bundle/nsis/*.exe
```

## 改一处 → 发布到桌面

```bash
pnpm check             # cargo test + 类型检查，提交前跑
pnpm release           # 测试 → 编 release exe → 重启桌宠（scripts/release.ps1）
```

`pnpm release -- -SkipTests` 跳过测试只编译。日常启动用 `启动桌宠.cmd`（= `scripts/start-vpet.ps1`：拉起 Ollama、TTS、桌宠，缺模型会自动下载）。
每次 push 都会在 GitHub Actions 上跑同一套测试（`.github/workflows/ci.yml`）。

```bash
pnpm release:github -- -Version 0.2.0   # 发正式版：打包 NSIS 安装包 + 便携 zip，打 tag，建 GitHub Release
pnpm clean                              # 本机只留正式版：清掉 debug 构建、dist、旧的 TTS 构建和残留副本
```

发布前要 `gh auth login` 一次；不给 `-Version` 就把补丁号 +1，`-DryRun` 只打包不提交。安装包里带着角色美术，仅供个人使用。

想让她出声，再跑一次 `scripts/setup-tts.ps1`（用 VS Build Tools 自带的 CMake 编译 qwentts.cpp，下载 Qwen3-TTS 权重；可选）。

语义检索默认走本机 Ollama 的 embedding 接口（`qwen3-embedding:0.6b`），编译期不下载任何东西。
想完全离线、不依赖 Ollama 的可选 ONNX 后端：`pnpm dev:onnx`（会在编译期下载 ONNX Runtime）。

## 和她相处

- **单击**人物打开对话窗口；**按住拖动**把她搬到别处；**右键**打开设置。全局快捷键 `Alt+V` 也能呼出对话。
  把人物拖出屏幕左 / 右边超过约 50 个角色像素，她会用原版 `SideHide` 动画挂在边缘；鼠标移上去会探头，按下便完整回到屏幕内。
  真正空闲时她也会自己走路、爬行、爬墙或从高处落下；到屏幕边缘会按原版规则衔接方向兼容的动作，任何触摸、对话或状态变化都会立即打断移动。
  对话窗口拖到屏幕左右边缘会**吸附并自动收起**（只留一条边，鼠标碰到再滑出来）；自己发过的话悬浮可「重新发送」。
- 作息：23 点睡到早上 8 点（`actions.toml` 的 `sleep`），到点吃饭、上班、学习、玩；电脑待机再唤醒会把这段时间补上，不会早上还赖床。
- 她说的话浮在**头顶的气泡**里；「帮我设个番茄钟，学习一个小时」「十分钟后叫我」会在头顶挂一个**倒计时环**，到点提醒。
- 设置里可以调**显示大小**（200–800 px）、是否**始终置顶**，写她的**名字 / 背景 / 形象 / 性格 / 说话方式**，选一件**礼物**送她。
- 对话跑在本机 [Ollama](https://ollama.com) 上（默认 `qwen3.5:9b`，6.6 GB），不联网、不上传。为避免重复占用磁盘，启动器只维护当前配置的模型，不再随包保留第二套 4B 权重。
  `scripts/start-vpet.ps1`（或双击 `启动桌宠.cmd`）会自动拉起 `.runtime/ollama` 里的服务、补齐缺的模型并启动桌宠。
  脚本默认打开 Vulkan 核显推理（`OLLAMA_IGPU_ENABLE=1`）：在 Intel Arc 核显上 9B 的首字延迟从 4.8 s 降到 1.9 s，
  且推理不再占 CPU。内存紧张（其他程序占用超过 ~20 GB）时，对话模型和嵌入模型会被 Ollama 轮流换出，回复会偶尔多等几秒。
- **在对话里使唤她**：「去玩会儿」「休息 10 分钟吧」「该睡觉了」「go take a nap」——这些话会先进状态机过一次服从判定
  （她不一定答应：看心情、身体和好感度），答应了就真的去做，并且有一段**保护期**（做完这一轮，或你说的时长，最长四小时），
  作息不会中途把她拽走；过了保护期她照自己的安排继续。「多工作一点」「别玩了」则是持续几小时的倾向。她的回复会知道自己刚答应了 / 拒绝了什么。
- **她会说话**：动作开始时随口一句（去工作、开饭、拆礼物……每个动作可以在设置里配多条台词，也可以让本机模型按她的性格写几条或每次即兴），
  答应 / 拒绝你、收到礼物、对话回复都能念出来。语音是本机的 [Qwen3-TTS](https://github.com/QwenLM/Qwen3-TTS)
  （0.6B CustomVoice，中英文都行；默认声音 `vivian`，使用偏快、偏高、起伏较小的 Neuro 风预设），通过
  [qwentts.cpp](https://github.com/ServeurpersoCom/qwentts.cpp) 的 GGML 移植在核显上跑（Vulkan，约 0.75 倍实时），不联网。
  第一次运行 `scripts/setup-tts.ps1` 编译并下载权重（约 1.3 GB，全部落在 `.runtime/`），之后 `start-vpet.ps1` 会自动拉起 `tts-server`。
  设置页「声音与台词」可以关掉语音、换声音、试听、逐个动作改台词。
  自动动作台词在 23:00–08:00 静音；提醒、你主动发起的对话和送礼回应不受影响。同一动作短时间来回切换也不会复读。
- 觉得某句回答好就点 👍，不好就写下「你希望她怎么说」；这些样本可在设置里导出，
  用 [training/](training/README.md) 里的 LoRA 脚本训练成你自己的模型，再在设置里切换过去。
  聊天记录只是对话，**不会**自动进入长期记忆；说「记住：…」才会。

## 许可

代码 Apache-2.0（见 LICENSE）。`assets-src/` 中的角色美术归原作者，个人使用；公开分发前需确认授权。
