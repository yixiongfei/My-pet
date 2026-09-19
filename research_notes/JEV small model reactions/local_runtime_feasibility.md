# 本机小模型反应选择：运行时可行性研究

> 研究日期：2026-09-19（日本时间）。目标：评估在 Windows 桌面 companion-agent 中，让小型本机语言模型选择反应/台词的可行性；不替代 Core 的确定性状态机。本文只记录研究结论，不修改代码。

## 结论摘要

- **可行，但应把模型放在“候选反应/措辞选择器”而不是决策真相的位置。** VPet 现有边界已经适合：Rust Core 计算状态、服从判定和合法动作；模型只从 Core 提供的有限候选中选择 `reaction_id`、语气和短台词。这样即使超时、卸载、输出不合法，也能回退到固定台词。
- **首选 Ollama 本机 HTTP 服务**：Windows 原生运行、无需管理员安装、`127.0.0.1:11434`，Rust 用现有 `reqwest`/异步任务调用；模型常驻 (`keep_alive`) 可避免每次重新加载。若需要单文件/嵌入式分发，再评估 llama.cpp server 或 ONNX Runtime GenAI。
- **首选模型试验梯度**：`gemma3:1b`（极低资源、文本）、`qwen3:0.6b`（中文/多语言与指令跟随优先）、`qwen3:1.7b` 或 `qwen3:4b`（质量优先）、`SmolLM2:1.7B`（轻量基线）。Qwen3 的 thinking 必须关闭/限制，否则反应选择的延迟和输出长度不可控。
- **结构化输出可做成硬约束**：Ollama `format` 接受 `json` 或 JSON Schema；llama.cpp 支持 GBNF grammar，server/CLI 可用 grammar 或 JSON schema 约束。仍需 Rust 端 schema 校验、枚举白名单和超时回退。
- **不要把高频每秒心跳交给模型。** 对话/用户触发/跨病线/收礼等低频事件才调用；状态机每秒 tick、动作合法性、工具权限仍纯规则。

## 1. Windows 桌面与运行时选型

### Ollama（推荐第一阶段）

- Ollama Windows 是原生应用，支持 NVIDIA 与 AMD Radeon；要求 Windows 10 22H2 或更新版本。API 默认在 `http://localhost:11434`，本地调用无需 API key。[Ollama Windows 文档](https://docs.ollama.com/windows)
- 安装器默认无需管理员权限；二进制约需至少 4 GB，模型空间另计；可用 `OLLAMA_MODELS` 把权重移到数据盘。文档明确模型可能占用数十到数百 GB，因此产品设置应显示模型名称/磁盘占用并允许用户选择目录。[Windows 文件系统要求](https://docs.ollama.com/windows#filesystem-requirements)
- Ollama 可以作为独立 zip/服务运行（`ollama serve`），这适合 Tauri 启动脚本；但要处理已有 Ollama 进程、端口占用、首次模型下载、GPU 驱动和用户关闭托盘进程等生命周期问题。[Standalone CLI](https://docs.ollama.com/windows#standalone-cli)
- Ollama API 文档列出本地 `/api` 与 OpenAI-compatible `/v1` 基址；API 不严格版本化但承诺兼容性，Rust 端应固定请求字段并容忍新增字段。[API introduction](https://docs.ollama.com/api/introduction.md)

**对 VPet 的含义**：保持现有 `chat::local_endpoint` 校验（只允许 loopback），把 reaction 请求和聊天请求复用同一个本机服务；Core 中用后台 Tokio 任务，不能阻塞 `reduce` 或每秒心跳。启动时探测 `/api/tags`/模型是否已拉取，失败立即使用规则台词。

### llama.cpp（更可控、分发更复杂）

- llama.cpp 的 GBNF 是用于约束模型输出的正式语法，可强制生成合法 JSON 或其他语言；语法支持 CLI、completion 和 server。[GBNF Guide](https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md)
- GGUF + llama.cpp 适合将量化模型作为随应用管理的文件，避免依赖 Ollama 安装；代价是 Windows GPU backend（CUDA/Vulkan/新驱动）、模型下载/更新、进程崩溃和 DLL 打包要自行维护。建议先把它作为“可选高级运行时”，不要与 Ollama 同时成为首发路径。
- GBNF/JSON grammar 是 token 级约束，不是业务验证；grammar 只能保证语法形状，不能保证 `reaction_id` 是允许的动作。因此 Rust 仍必须反序列化、枚举校验、长度限制和默认回退。

### ONNX Runtime GenAI（Windows/硬件集成候选）

- ONNX Runtime Generate API 封装了 tokenization、推理循环、logits 处理、采样和 KV cache，并提供 greedy/beam/top-k/top-p 等能力；文档标注 API **preview、可能变化**，并提到 structured output（tool calling）。[ONNX Runtime GenAI](https://onnxruntime.ai/docs/genai/)
- 官方仓库提供 C++/C#/Python 等样例与 NuGet 管理包；Windows DirectML/CUDA/CPU 适配有吸引力，但模型转换、EP 选择和 Rust FFI/sidecar 复杂度显著高于 Ollama。[onnxruntime-genai README](https://github.com/microsoft/onnxruntime-genai)
- 结论：若以后需要单进程、可控内存和 Windows NPU/DirectML，做独立 benchmark 分支；当前 Rust + Tauri 原型优先 HTTP sidecar。

## 2. 结构化 JSON 的可靠性与接口设计

### Ollama

- Ollama structured outputs 的 `format` 可以传 `"json"`，或直接传 JSON Schema；官方建议同时把 schema 作为 prompt 的一部分，并在客户端用 Pydantic/Zod 等再次验证。[Ollama Structured Outputs](https://docs.ollama.com/capabilities/structured-outputs)
- 官方示例用 `stream:false` 取完整 JSON，并建议低 temperature（例如 0）获得更确定结果；这很适合反应分类，不适合自由聊天的温度参数。[同上](https://docs.ollama.com/capabilities/structured-outputs)
- Structured output 只约束输出格式，模型仍可能选择语义上错误的值；`reaction_id` 应是有限枚举，Rust 端用 `serde` + `deny_unknown_fields`/长度限制，拒绝额外字段和超长文本。对失败响应记录 reason，但不把原始文本直接执行。

建议的最小契约（示意，不写入代码）：

```json
{
  "reaction_id": "comfort|accept|decline|celebrate|idle",
  "tone": "warm|playful|tired|firm",
  "line": "最多 80 个 UTF-8 字符的短句",
  "confidence": 0.0
}
```

实际 schema 应把 `reaction_id`/`tone` 写成 enum，`line` 设置 `maxLength`，`confidence` 限制 0–1；Core 再检查该候选是否与当前 `Verdict`、健康/病重状态一致。模型不应返回动作参数、工具名、文件路径或任意代码。

### llama.cpp grammar / JSON schema

- GBNF 可描述 JSON 字面量、数组、对象和枚举；使用 grammar 时应为 reaction schema 生成专用 grammar，而不是宽泛的“任意 JSON”。[GBNF Guide](https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md)
- 约束解码会增加每 token 的解析工作，且短输出通常影响很小；实际开销要在目标 CPU/GPU 和 grammar 复杂度上测量。JSON valid ≠ 业务 valid，仍需 serde/schema 校验。

### 推荐失败策略

1. 发送小上下文：Core 状态摘要 + 事件 + 允许候选 ID，不发送完整聊天历史。
2. `temperature=0`（或极低）、`stream=false`、短 `num_predict`/token 上限、请求超时。
3. 解析失败/超时/模型未加载/服务不可用：立即选确定性候选；不要重试多次造成 UI 卡顿。
4. UI 通过 `chat:thinking` 或专用 `reaction:thinking` 表示等待，成功后发 `pet:line`；Body 不解析模型自由文本做业务判断。

## 3. 适合本机反应选择的小模型（截至 2026-09）

以下是有官方模型卡/官方 Ollama 库页面的可选集。大小是参数规模，不等于最终磁盘大小；实际 GGUF/Ollama 层还受量化、KV cache、上下文影响。

| 模型 | 官方事实与适用性 | 反应选择建议 |
|---|---|---|
| **Qwen3 0.6B / 1.7B / 4B** | Qwen3 模型卡强调多语言、指令跟随、角色扮演和 agent 能力；官方卡：[Qwen3-0.6B](https://huggingface.co/Qwen/Qwen3-0.6B)。Ollama 页面提供 Qwen3 家族及运行方式：[Ollama qwen3](https://ollama.com/library/qwen3) | 中文短句/情绪标签的优先候选。用 non-thinking；0.6B 适合极低延迟，1.7B/4B 用于更复杂上下文。必须实测中文遵循 schema 的失败率。 |
| **Gemma 3 270M / 1B** | Ollama 页面列出 270M、1B 文本模型（及更大多模态），1B 文本上下文 32K；还列出 QAT 版本，称在较低内存下接近 BF16 质量。[Ollama Gemma 3](https://ollama.com/library/gemma3)；模型卡：[Gemma 3 1B IT](https://huggingface.co/google/gemma-3-1b-it) | 270M/1B 是启动速度和内存基线；中文角色表现需单独验收。1B-it-qat 可作为内存受限的试验。 |
| **SmolLM2 135M / 360M / 1.7B** | Hugging Face 官方集合和 Ollama 页面明确提供三种尺寸，定位为可在设备端运行的 compact models。[Ollama SmolLM2](https://ollama.com/library/smollm2)，[SmolLM2-1.7B-Instruct](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct) | 英文/简单分类的极轻基线；中文或复杂角色一致性不应先验假定，需测试。 |
| **Llama 3.2 1B Instruct** | Meta 模型卡说明有 1B/3B text-in/text-out、instruction-tuned、多语言对话和 agentic retrieval/summarization 目标。[Llama 3.2 1B](https://huggingface.co/meta-llama/Llama-3.2-1B-Instruct) | 生态/工具链成熟的对照组；注意许可条款、中文质量和 Ollama 具体 tag 的可用性。 |
| **Phi-4-mini-instruct 3.8B** | Microsoft 模型卡提供 mini-instruct 与 ONNX 相关链接，定位为小型指令模型；[模型卡](https://huggingface.co/microsoft/Phi-4-mini-instruct) | 质量可能更好但内存/延迟高于 1B 级，适合有 GPU 的可选档，不是低端机默认。 |

**量化约束（工程估算）**：权重裸大小可用 `参数数 × bits/8` 粗估，另加 scales/metadata、KV cache 和运行时开销；因此 4-bit 1B 不是恰好 0.5 GB，且长上下文会增加 KV。Q4_K/Q5_K 等 GGUF/Ollama 量化应以实际 `ollama ps`、进程工作集和首 token/总耗时为准。模型卡的 benchmark 不能直接当作 VPet 延迟，因为 prompt 长度、GPU offload、线程数、驱动和 JSON grammar 都不同。

## 4. 延迟、量化与资源预算

### 影响延迟的组成

- **冷启动/加载**：首次请求把权重映射/上传到 GPU，通常远高于后续 token 生成；必须区分 cold 与 warm 测试。
- **prefill**：输入状态摘要和 schema 的 token 数；上下文越长，首 token 越慢。
- **decode**：输出 token 数；reaction 只需结构化对象和 1 句短台词，应把上限压到约 64–128 tokens，而非开放式聊天。
- **offload/驱动**：CPU-only、Vulkan、CUDA、DirectML/NPU 的吞吐差异很大；Windows GPU 驱动版本是外部变量。Ollama Windows 文档列出 NVIDIA 551.61+ 与 AMD ROCm/Vulkan 条件。[Windows requirements](https://docs.ollama.com/windows#system-requirements)
- **约束解码与 JSON**：schema/grammar 让结果可验证，但增加少量 token 处理；必须在目标机器测量。

### 可执行的验收目标（项目约束，不是厂商保证）

- warm 请求首 token P95 ≤ 300 ms、完整 reaction P95 ≤ 1 s（有 GPU）；CPU-only 允许放宽到 P95 ≤ 2 s，并且 UI 有规则台词即时回退。
- 模型加载不在 Core 心跳线程；单次 reaction deadline 约 1–2 s，超时后取消 HTTP 请求并回退。只允许一个并发 reaction，避免小模型排队拖住桌宠。
- 对每个候选模型记录：冷/热首 token、总耗时、tokens/s、峰值 RSS/VRAM、schema 解析失败率、中文/日文/英文各 100 条测试、重复率和拒绝一致性。
- 先测 `gemma3:1b`、`qwen3:0.6b` 和 `qwen3:1.7b` 的 Q4/Q8（若 tag 可用），再决定是否上 4B。不可根据参数量单独推断体验。

## 5. 与 Rust Core + React/Tauri 的落地边界

现有设计已经给出明确安全边界：[组件说明](https://github.com/yixiongfei/My-pet/blob/main/docs/02-components.md)、[Core 规则](https://github.com/yixiongfei/My-pet/blob/main/docs/03-core.md)、[Brain 说明](https://github.com/yixiongfei/My-pet/blob/main/docs/05-brain.md)。其中 Core 是唯一接本机服务的层，Body 只演，`reduce` 是纯函数，用户意志必须走 `request_action`。

建议的 reaction pipeline：

1. `reduce`/事件产生确定性 `ReactionContext`：事件类型、状态摘要、合法候选、是否病重、现有 `Verdict`；不让模型重新判断健康、服从或工具权限。
2. Rust `reaction_selector` 后台调用 Ollama `/api/chat`，`format` 传 JSON Schema；设置低温度、短上下文、短输出和 deadline。
3. Rust 用 `serde_json` 严格解析，检查 enum、长度、候选集合、状态一致性；失败调用纯函数 `fallback_reaction`。
4. 只把已验证的 `{text, action, spoken, level, volume}` 通过 Tauri event 发给 React；React 不直接访问 Ollama，也不执行模型返回的动作。
5. 对话仍可走现有流式 `chat-stream`；reaction 选择器应使用独立请求/并发限制，避免聊天占满模型时阻塞台词。
6. 记录本地诊断指标（模型 tag、冷/热、耗时、回退原因、token 数），不要上传内容；用户可关闭模型 reaction 并回到固定台词。

### 不适合交给小模型的内容

- 状态数值更新、`reduce` 合法转移、健康阈值、病重禁行动。
- 服从骰子、`request_action`、工具调用/权限、文件/网络访问。
- 每秒心跳、后台调度和必须及时的生病/药物安全逻辑。

### 适合交给小模型的内容

- 在已确定的 `Verdict` 后选择同义短台词、语气、emoji/动画 hint（仍用 enum）。
- 低频主动开口的措辞改写；失败就用 actions.toml 固定句。
- 对话回复中的有限 reaction metadata；不要把正文解析为命令。

## 6. 风险、缺口与下一步实验

- **模型版本/标签会变**：Ollama 库页面是滚动目录，发布时锁定 tag/digest，并保留模型许可与来源清单。
- **结构化输出兼容性要实测**：官方 Ollama 文档说明能力，但不同模型的 chat template、thinking 模式和小参数规模可能导致拒绝/重复；测试 `format` schema、`stream:false`、temperature 0 和停止条件。
- **中文/日文角色质量没有统一可比的官方数字**：模型卡 benchmark 不能替代 VPet 数据集；建立包含“饿/病/被拒/收礼/用户使唤”等固定回归集。
- **Windows 低端硬件差异很大**：CPU 型号、内存、GPU 驱动和后台进程必须在至少 CPU-only、NVIDIA、AMD/Vulkan 三类机器上测；不要承诺单一毫秒数。
- **最小 PoC**：不改生产代码，写独立 Rust/PowerShell harness，固定 100 个 ReactionContext，分别测 Ollama Qwen3/Gemma/SmolLM2；输出 JSON 解析率、P50/P95、峰值内存和回退率。通过后再决定是否把 selector 接进 `lines.rs`/`chat.rs`。

## 参考链接清单

- [Ollama Windows](https://docs.ollama.com/windows)
- [Ollama API](https://docs.ollama.com/api/introduction.md)
- [Ollama Structured Outputs](https://docs.ollama.com/capabilities/structured-outputs)
- [llama.cpp GBNF Guide](https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md)
- [ONNX Runtime Generate API](https://onnxruntime.ai/docs/genai/)
- [ONNX Runtime GenAI repository](https://github.com/microsoft/onnxruntime-genai)
- [Qwen3-0.6B model card](https://huggingface.co/Qwen/Qwen3-0.6B)
- [Ollama Qwen3 library](https://ollama.com/library/qwen3)
- [Ollama Gemma3 library](https://ollama.com/library/gemma3)
- [Gemma 3 1B model card](https://huggingface.co/google/gemma-3-1b-it)
- [Ollama SmolLM2 library](https://ollama.com/library/smollm2)
- [SmolLM2-1.7B model card](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct)
- [Llama 3.2 1B model card](https://huggingface.co/meta-llama/Llama-3.2-1B-Instruct)
- [Phi-4-mini-instruct model card](https://huggingface.co/microsoft/Phi-4-mini-instruct)
- [VPet component boundaries](https://github.com/yixiongfei/My-pet/blob/main/docs/02-components.md)
- [VPet Core rules](https://github.com/yixiongfei/My-pet/blob/main/docs/03-core.md)
- [VPet Brain](https://github.com/yixiongfei/My-pet/blob/main/docs/05-brain.md)
