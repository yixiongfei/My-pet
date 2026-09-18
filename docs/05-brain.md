# 05 · Brain：对话、记忆、声音

代码：`src-tauri/src/chat.rs`（对话）· `core/intent.rs`（规则意图）· `core/memory.rs` / `embed.rs` / `db.rs`（记忆）· `tts.rs`（语音）· `lines.rs`（台词）。
和最初设计最大的出入：**Brain 在 Rust 里，走本机 Ollama，没有 tool calling**。本机 9B 模型调工具不可靠且慢，而高频意图（去玩、番茄钟、提醒）用规则几毫秒就能认出来，模型只负责把结果说得像她。`packages/brain` 是空壳。

## 1. 对话（`chat.rs`）

- 模型 `qwen3.5:9b`，`http://127.0.0.1:11434`，temperature 0.75；地址经 `local_endpoint` 校验只接本机。设置里可换模型（含自己训练的）。
- **系统提示** = 角色卡（名字 / 背景 / 形象 / 性格 / 说话方式）+ 短规则 + 「此刻」（在做什么、心情、饿渴累、**病没病**、刚答应 / 拒绝了什么）+ 召回的记忆。
- **上下文**：最近 8 轮、≤ 10k 字；失败 / 取消 / 差评的回答不进；重复的相同提问去重（重复的用户轮会让模型提前 EOS 截断）。
- **预填充 `萝莉斯：`** 压住 9B 的推理泄露；`<think>` 段剥掉；停止串 `用户：` `[旁白`。
- **清理**：去名字前缀、去括号里的舞台指示、去掉未闭合的括号尾巴；被打断 / 截断时补「…」。
- **没进角色就重来一次**：开头 80 字里堆了两个以上分析词（「我需要」「用户想」「作为虚拟伙伴」…）判为模型在盘算，向用户消息追加 `[旁白：直接开口]` 重生成一次，流里已出去的字用 `reset` 作废。
- 👍 / 修订 → `export_training_data` 导出 JSONL；`training/` 里的 LoRA 脚本（本机无 CUDA，未训练过）。

## 2. 规则意图（`intent.rs`）

对话先过一遍规则，认出来的直接进 Core，**模型不能自己改状态**：

| 说法 | 意图 | 去向 |
|---|---|---|
| 去玩会儿 · 休息 10 分钟 · 该睡觉了 · go take a nap | `Do { target, minutes }` | `request_action` → 服从判定 → 保护期 |
| 多工作一点 · 别玩了 · study more | `Bias { tag, weight }` | `set_bias`（半衰期 120 min） |
| 帮我设个番茄钟，学习一个小时 | `Focus { minutes, target }` | 专注段 + 头顶倒计时 |
| 十分钟后叫我 | `Timer { minutes, label }` | `create_timer` |
| 记住：… · 忘记… · 你记得我什么 | 记忆命令 | `remember` / `forget` / `recall` |

解析顺序 focus → timer → bias → do；中文先于英文。规则**偏保守**：疑问句（「要不要去玩？」）、过去式（「去玩了吗」）、没有对她说的（「我去睡了」）一律不认——误把闲聊当命令比漏掉一句糟。判定结果写进「此刻」和用户消息后的 `[旁白：…]`，模型顺着说；只有那里写了，她才说自己去做了。

## 3. 长期记忆（`memory.rs`）

存的不是聊天记录，是**可复用的结论**（「用户在准备 2027 考研」）。聊天是流水，记忆是索引，两者生命周期相反——把流水当索引用，检索质量会随使用时间单调下降。

- **表** `memory_items`（迁移 v3）：content · type · importance 0–100 · confidence 0–1 · source · pinned · 时间戳 · access_count · expires_at · status · embedding_version。**唯一真相来源**，向量索引只是加速器。
- **六种类型**：profile（身份与目标）· preference · habit · temporary_context（有效期）· relationship · commitment。
- **来源即优先级**：`user_explicit` > `user_confirmed` > `system_event` > `inferred`。**推断不能当事实**——`plan_write` 对 `Inferred` 只回 `NeedsConfirm`，类型上堵死；敏感内容同理。
- **写入三条硬规则**：推断要确认；纠正不是新增（相似度 ≥ 0.55 的旧条目被取代，软删除留审计）；普通闲聊不写——只有「记住：…」才写。
- **检索打分**：`0.55 语义 + 0.20 重要性 + 0.10 新鲜度 + 0.10 使用频次 + 0.05 置信度`，取前 5 条进系统提示；置顶必进；过期 / 归档 / 未确认的过滤掉；冲突按来源优先级消解，不靠模型。
- **语义向量**（`embed.rs`）：默认本机 Ollama `/api/embed`（`qwen3-embedding:0.6b`，和对话模型共用运行时）；可选 ONNX 后端（`embed` feature，`ort` + `tokenizers`，不用 fastembed）。模型缺了退回字符 bigram 字面检索，宠物照跑。索引在 sqlite-vec `vec0`；换模型整张重建并后台补算。混合检索是两路余弦的加权和，稠密侧的门槛是「在不在 KNN 结果里」，不用绝对阈值。
- 面板：列表 / 检索预览 / 置顶 / 归档 / 删除 / 健康度。

## 4. 声音（`tts.rs`）

- **Qwen3-TTS 0.6B CustomVoice** 通过 [qwentts.cpp](https://github.com/ServeurpersoCom/qwentts.cpp)（C++ / GGML）在核显上跑：用 Ollama 自带的 `ggml-vulkan.dll`，`tts-server` 在 `127.0.0.1:8090`（OpenAI 兼容 `/v1/audio/speech`），RTF ≈ 0.75。`scripts/setup-tts.ps1` 编译 + 下载权重（1.3 GB，落 `.runtime/`），一次性。
- 默认声音 `vivian`，Neuro 风预设：「语气平稳、起伏小，节奏偏快，音调偏高」；语速 1.12（`playbackRate`，不保音高所以顺带更高）；中英文都能念。
- **按句念**：Body 的 `Speech` 队列把回复按句切开，前一句在放时后一句已在合成；出声期间播 `say` 动画；气泡念完才收。
- 文本先清理（去括号 / markdown / emoji，≤ 300 字），按 `声音|语气|文本` 哈希缓存在 `tts-cache/`。
- 念什么：对话回复、动作台词、答应 / 拒绝、收礼、生病 / 康复。设置页「声音与台词」可关、换声音、试听。

## 5. 台词（`lines.rs`）

- 开始做一件事时随口一句：`actions.toml` 每个动作带 `lines`（`{food}` `{name}` 模板），设置里可逐个动作改成多条、或让本机模型按她的性格写几条 / 每次即兴。
- 总开关 + 三种模式（固定 / 模型 / 关）；同一动作短时间来回切不复读（间隔 45 s）；自动动作台词 23:00–08:00 静音，提醒、对话、送礼、生病不受影响。
- 不挂在动作上的话（跨过病线、没病不用吃药）走 `lines::say`，不受间隔限制。
