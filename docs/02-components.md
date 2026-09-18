# 02 · 核心组件结构

一个 Tauri 进程、三个 WebView、一层 Rust。**Core 决定一切，Body 只演，Brain 只说。**

## 1. 核心组件结构

```
VPet.Core（Rust，apps/desktop/src-tauri/src）
├── lib.rs               （窗口 · 托盘 · 穿透命中 · 心跳 · 全部 Tauri 命令）
├── core/                （确定性核心：不读时钟、不做 IO 的纯逻辑）
│   ├── state_machine    （数值与状态：reduce(state, event)）
│   ├── actions (.toml)  （动作表：消耗 / 收益 / 门槛 / 时段 / 台词）
│   ├── obey             （服从判定：对数几率）
│   ├── bias             （偏好权重：带半衰期）
│   ├── intent           （规则意图：中英文的使唤 / 番茄钟 / 提醒）
│   ├── scheduler        （计时器 · 专注段）
│   ├── pomodoro         （番茄钟相位机）
│   ├── food             （JSON 货架：按需求随机吃喝 · 礼物 · 药单）
│   ├── memory · embed   （长期记忆 · 语义向量）
│   ├── db               （SQLite：状态流水 · 记忆 · 向量 · 计时器）
│   └── tools            （工具注册 · 权限门 · 审计）
├── chat.rs              （对话：角色卡 + 此刻 + 记忆 → 本机 Ollama 流式）
├── tts.rs               （语音合成 + 缓存）
├── lines.rs             （动作台词：固定 / 模型即兴）
├── kb.rs                （本机知识库只读：当天日程 · 到期复习，读它的 .kb/index.db）
├── nudge.rs             （主动提醒：确定性规则，一句话、按天记账、人在才说）
├── pet_motion.rs        （自主移动 · 侧挂 · 窗口几何 · 出屏弹回）
├── dock.rs              （对话窗口贴边收起）
└── desktop_settings.rs  （显示大小 · 置顶）

VPet.Body（React，apps/desktop/src）
├── body/                （宠物窗口）
│   ├── AnimationPlayer  （Canvas 三段式播放器 · 夹心双图层 · 心情降级）
│   ├── interaction      （交互状态机：摸 / 提起 / 说话 / 移动 / 侧挂）
│   ├── animationPool    （行为树「当前计划」层：同类动画按驻留轮换）
│   ├── PetCanvas        （头顶区布局 · 事件订阅）
│   ├── Bubble · Countdown （气泡 · 倒计时环）
│   ├── speech           （按句 TTS 队列）
│   └── hitMask          （alpha 命中掩码）
├── chat/                （对话窗口：记录 · 输入 · 👍/👎 · 重新发送）
└── panel/               （设置：陪伴 · 个性 · 声音 · 模型 · 礼物；同风格的折叠状态与调试区）

VPet.Shared（packages/shared，zod）
└── PetState · ActionRef · Verdict · Manifest · Memory   （TS 与 Rust 共用的 JSON 契约）

scripts/
├── build-assets.mjs     （convert:pet / convert:food：LPS → JSON；build:assets：JSON + PNG → WebP / manifest / Core 货架）
├── start-vpet.ps1       （拉起 Ollama · tts-server · 桌宠）
├── make-shortcut.ps1    （桌面「VPet 桌宠」快捷方式 → start-vpet.ps1，不复制 exe）
├── setup-tts.ps1        （编译 qwentts.cpp、下载权重，一次性）
└── release.ps1 · publish-release.ps1 · clean.ps1

schemas/
├── pet-source-v1.schema.json  （vup.json：角色 profile · 动画映射 · 帧时长 · 夹心轨迹）
└── food-source-v1.schema.json （food.json：礼物 · 食物 · 饮料 · 药物）
```

## 2. 边界规则

1. **Body 不做业务逻辑。** 数值、台词、动作、窗口位移都由 Core 决定后用事件推过来；Body 只有「怎么演」的自由（动画池、气泡行数）。
2. **用户意志进状态机只有一个入口**：`request_action` → 服从判定。对话里的「去玩会儿」也走这里，模型只是把结果说出来。
3. **Core 是唯一碰网络和数据库的地方。** Ollama / tts-server 的地址都经 `chat::local_endpoint` 校验，只接本机。
4. **`reduce` 是纯函数。** 时间以 `Event::Tick { minutes, hour }` 喂，随机数以 `roll` 喂，所以作息、保护期、服从概率、健康都有单元测试。
5. **前后端各一份的常量成对改**：`HEAD_ROOM`、`SICK / ILL`、`SPEAKERS`、默认设置。

## 3. 契约：命令与事件

Body → Core（`invoke`，节选）：

| 类别 | 命令 |
|---|---|
| 状态 | `get_pet_state` `pet_touched` `request_action` `get_directive` `debug_patch_pet_state` |
| 照顾 | `give_gift` `list_gifts` `give_medicine` `list_medicines` `set_food_catalog` |
| 日程 | `kb_agenda`（今天的日程）`say_agenda`（让她说一遍今天的安排） |
| 时间 | `create_timer` `cancel_timer` `list_timers` `start_pomodoro` `stop_pomodoro` `start_focus_session` `cancel_focus` `get_focus` |
| 对话 | `send_chat_message` `cancel_chat` `list_chat_messages` `rate_chat_message` `get_chat_settings` `save_chat_settings` `export_training_data` |
| 声音 | `tts_speak` `tts_status` `list_actions` `draft_action_lines` |
| 记忆 | `remember` `forget_memory` `search_memory` `memory_context` `list_memories` `pin_memory` `memory_health` `rebuild_embeddings` |
| 偏好 / 工具 | `set_bias` `clear_bias` `list_biases` `list_tools` `run_tool` `recent_audit` |
| 窗口 | `set_hit_mask` `set_hit_test_pinned` `begin_pet_drag` `end_pet_drag` `pet_motion::*` `open_chat` `open_settings_panel` `request_shutdown` `finish_shutdown` |

Core → Body（`emit`）：

| 事件 | 载荷 | 何时 |
|---|---|---|
| `pet:state` | `PetState` | 每次状态机跑过一拍且有变化（也定期同步） |
| `pet:line` | `{text, action, spoken}` | 开始做一件事的台词、生病 / 康复、没病不用吃药、日程提醒（`action = "nudge"`） |
| `pet:said` | `Verdict` | 一次服从判定的结果 |
| `pet:gift` | `{id, name}` | 收到礼物，拆一遍 |
| `pet:motion` | 移动指令 | 自主走路 / 爬墙 / 侧挂 |
| `pet:shutdown-requested` | — | 托盘退出：Body 播退场后回调 `finish_shutdown`；Core 8 秒兜底 |
| `chat:thinking` | `{requestId, active}` | 模型首字前开始 / 取消或失败时结束思考动画 |
| `chat-stream` | `{requestId, delta, done, reset?}` | 对话流式输出 |
| `focus:started` / `focus:ended` / `timer:fired` | 计时器 | 头顶倒计时、到点提醒 |
| `pomodoro:tick` / `pomodoro:phase` | 番茄钟 | 相位推进 |
| `chat:settings-changed` `desktop:settings` `embed:state` `audit:appended` `tool:confirm` | — | 设置 / 索引 / 审计变化 |

## 4. 数据流

**心跳**（每秒，`spawn_pet_clock`）：按墙上时钟算这一拍多长（待机醒来补算，封顶 120 分钟）→ `reduce(Tick)` → 换了件事就 `lines::announce` → 跨过病线就说一句 → `pet:state`。

**你说话**：对话窗 → `send_chat_message` → `intent::parse`（规则）→ 认出使唤就 `request_action`（服从判定）/ 番茄钟 / 计时器 / 偏好 → 结果写进系统提示「此刻」和用户消息后的 `[旁白]` → `chat:thinking` 播思考 → Ollama 首字结束思考 → `chat-stream` → 气泡 + 按句 TTS。模型**不能**自己改状态。

**你照顾她**：托盘「送她礼物」→ 面板礼品页选定 id → `give_gift`；喂药走 `give_medicine`。两者再进 `Event::Gifted` / `Event::Medicated` → 收礼 / 吃药夹心动画 → 台词。

## 5. 数据与目录

- 数据在 `%APPDATA%\dev.yixiongfei.vpet\`：`vpet.db`（状态流水、记忆、向量、计时器）、`chat.sqlite3`（对话与设置）、`desktop-settings.json`、`tts-cache/`。
- 运行时在 `.runtime/`（Ollama、tts-server、模型权重，不入库）；美术在 `assets-src/`（不入库）；正式版 exe 只在 `src-tauri/target/release/`。
- 只有一个远程 `origin`（github.com/yixiongfei/My-pet）；CI 跑 cargo test + typecheck。

## 6. 安全与隐私

- 只接本机地址；不联网、不上传；对话记录不自动进长期记忆（说「记住：…」才会）。
- Brain 动系统只有一条路：`run_tool` → 权限门 → 执行 → 审计。`Ask` 目前当拒绝处理（确认气泡未做）。
- 角色美术归原作者，个人使用；安装包不公开分发。
