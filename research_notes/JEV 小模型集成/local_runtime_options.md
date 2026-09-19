# VPet 本机小模型运行时选项

研究日期：2026-09-19（日本时间）  
目标硬件：Windows 11、Intel Core Ultra 5 125H、Intel Arc iGPU、32 GB RAM；现有 Ollama 与 qwentts.cpp。  
范围：只研究本机反应选择器（从 Core 给出的有限候选中选 `reaction_id`、语气和短台词），不把模型当作状态机、权限层或事实来源。

## 1. 先核对旧笔记

`research_notes/JEV small model reactions/local_runtime_feasibility.md` 的总体架构判断仍成立：模型应是可超时、可替换的候选选择器，Core 保持确定性，失败时使用固定台词。以下内容需要更谨慎地表述：

- **“Ollama 支持 Intel Arc”不能直接当作本机性能承诺。** Ollama Windows 文档明确列出 Windows、CPU，以及 NVIDIA/AMD 的支持条件；Intel Arc 的可用后端、驱动版本和具体性能应在目标机器上实测，不应把 NVIDIA/AMD 条目推演成 Arc 保证。[Ollama Windows](https://docs.ollama.com/windows)
- **Ollama `format`/llama.cpp grammar 保证的是语法，不是业务正确性。** JSON 合法并不保证 `reaction_id` 属于当前 Core 允许集合；仍需 Rust 反序列化、枚举白名单、长度限制和状态一致性校验。[Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs)；[llama.cpp GBNF](https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md)
- **旧笔记的毫秒目标是项目验收目标，不是厂商保证。** Core Ultra 5 125H、Arc 驱动、模型量化、上下文长度、后台负载和冷/热状态都会改变结果；必须报告 P50/P95、首 token 和完整响应。
- **“Q4 约等于参数数×0.5 GB”只能作下界估算。** GGUF/Ollama 层还包含量化元数据、张量对齐、KV cache、上下文和运行时工作集；以 `ollama ps`、系统工作集和实测峰值为准。
- **ONNX Runtime GenAI 是候选而非当前默认。** 它的 Generate API 仍标为 preview，模型转换、Execution Provider 选择和 Rust FFI/sidecar 生命周期会显著增加维护面。[ONNX Runtime Generate API](https://onnxruntime.ai/docs/genai/)

## 2. 运行时事实与建议

### Ollama：第一阶段首选

**已核实事实**

- Ollama 在 Windows 提供原生应用/CLI，API 默认在本机 `http://localhost:11434`；可用 `/api`，也提供 OpenAI-compatible `/v1`。[Windows](https://docs.ollama.com/windows)；[API introduction](https://docs.ollama.com/api/introduction)
- Structured Outputs 的 `format` 可传 `"json"` 或 JSON Schema；官方示例建议将 schema 也放进提示词、使用较低 temperature，并由客户端再次验证。[Structured outputs](https://docs.ollama.com/capabilities/structured-outputs)
- 模型驻留时间可通过 `keep_alive` 控制；这能避免每次反应重新加载权重，但会与聊天模型、TTS 和桌面程序竞争内存。请求应使用 `stream:false`、短上下文和短输出。[Generate API](https://docs.ollama.com/api/generate)
- Ollama 是独立进程。它的崩溃、升级、端口占用、模型下载和正在运行的聊天请求不会自动服从 Tauri/Rust 的生命周期，因此应用需探测服务、设置超时并准备回退。

**建议**

首版复用现有 Ollama，通过 Core 后台任务发独立 reaction 请求；只接受 loopback 地址。每个事件最多一个并发请求；1–2 秒 deadline 超时即回退，不在心跳线程重试。将模型 tag/digest 锁定到发布清单，并把聊天与 reaction 的驻留策略分开测量。

### llama.cpp / llama-server：可控的第二路径

**已核实事实**

- llama.cpp 使用 GGUF，并提供 CPU、Vulkan 等后端；`llama-server` 是本机 HTTP server。其 grammar/GBNF 可约束 JSON 等输出。[仓库 README](https://github.com/ggml-org/llama.cpp)；[Vulkan backend](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#vulkan)；[GBNF guide](https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md)
- grammar 是 token 级语法约束，不是候选集合权限系统；业务层仍要校验。
- 随应用分发 GGUF 可减少对用户 Ollama 安装的依赖，但要自行处理模型下载/更新、Vulkan 驱动、DLL、端口、崩溃重启和进程清理。

**建议**

把 llama-server 作为可选“便携/可控 runtime”，而不是首发同时维护两套默认路径。先用同一 GGUF、同一 prompt、同一 schema 与 Ollama 对比；Arc 上优先测试 Vulkan，同时保留 CPU fallback。CPU fallback 可能更慢但更容易复现，Vulkan 的实际收益必须在本机驱动上测量。

### ONNX Runtime GenAI / OpenVINO：仅在有明确集成目标时评估

**已核实事实**

- ONNX Runtime GenAI 封装 tokenizer、生成循环、logits 处理、采样和 KV cache，支持 CPU、CUDA、DirectML 等生态，但官方文档标注 API preview，接口和模型转换流程可能变化。[GenAI docs](https://onnxruntime.ai/docs/genai/)；[GitHub](https://github.com/microsoft/onnxruntime-genai)
- DirectML 是 Windows 图形设备的通用 EP；它不是 Intel Arc 专属性能保证。[DirectML EP](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)
- OpenVINO 面向 Intel CPU/GPU/NPU，提供 GenAI/LLM 示例和 Windows 包；但通常需要把模型转换为 OpenVINO IR/适配格式，并维护 Python/C++/sidecar 或 Rust FFI 集成。[OpenVINO GenAI](https://docs.openvino.ai/2025/learn-openvino/llm_inference_guide/genai-guide.html)；[OpenVINO install](https://docs.openvino.ai/2025/get-started/install-openvino.html)

**建议**

Core Ultra 5 的 NPU/Arc 让 OpenVINO 值得做独立 benchmark，但不是 Ollama 的无缝后端替换。若目标是最小交付风险，继续用 Ollama；若目标是 Intel 专用低功耗、单进程或 NPU 利用率，再建立 OpenVINO/ONNX sidecar 试验。没有官方端到端数据能证明本项目的特定模型在 Arc/NPU 上一定低延迟。

## 3. 模型与量化选择

| 候选 | 官方事实 | 本项目建议 |
|---|---|---|
| Qwen3 0.6B / 1.7B / 4B | Qwen3 模型卡覆盖多语言、指令跟随、角色扮演和 agent 场景，并说明可由 Ollama、llama.cpp 等部署；thinking 可切换。[Qwen3-0.6B](https://huggingface.co/Qwen/Qwen3-0.6B)；[Qwen3 library](https://ollama.com/library/qwen3) | 中文/日文短反应优先试 0.6B；质量与稳定性不足再试 1.7B。reaction 请求关闭 thinking，限制输出 token。4B 仅作为质量档。 |
| Gemma 3 270M / 1B | Google 模型卡提供 270M/1B 级文本模型；Ollama library 提供对应标签。[Gemma 3 1B](https://huggingface.co/google/gemma-3-1b-it)；[Ollama Gemma3](https://ollama.com/library/gemma3) | 作为低资源英文/分类基线；中文、日文和角色口吻必须实测。 |
| SmolLM2 135M / 360M / 1.7B | Hugging Face 将其定位为 compact on-device 模型，提供 instruct 变体。[SmolLM2](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct)；[Ollama](https://ollama.com/library/smollm2) | 极低资源基线；不先验承诺中文质量或 JSON 遵循。 |
| Llama 3.2 1B | Meta 模型卡提供 1B instruct 文本模型和成熟生态，但须遵守许可证。[Model card](https://huggingface.co/meta-llama/Llama-3.2-1B-Instruct) | 生态对照组，不是中文首选。 |
| Phi-4-mini 3.8B | Microsoft 提供小型 instruct 模型及 ONNX 相关资源。[Model card](https://huggingface.co/microsoft/Phi-4-mini-instruct) | 质量档/对照，不作为 32 GB 机器的最低延迟默认。 |

**量化与内存**

- 4-bit 的理论权重下界约为参数数×0.5 bytes；实际占用还包括量化元数据、KV cache、上下文、线程栈和 GPU/CPU staging buffer。8-bit 或更高精度通常更占内存，可能换取质量，但不能从参数量直接推断速度。
- 对反应选择，优先比较同一模型的 Q4_K（或 Ollama 对应量化）与更高精度版本；记录冷启动峰值、热运行工作集、Arc 显存/共享内存、首 token 和完整响应。避免长聊天历史，reaction 上下文只传状态摘要、事件和允许候选。
- 32 GB RAM 足以容纳 0.6B–4B 级量化模型与桌面应用的合理组合，但这是容量判断，不是并发/延迟保证；同时驻留 Ollama、TTS、浏览器和大模型可能造成分页。

## 4. 延迟、结构化输出与隔离

### 延迟预算（建议，不是事实）

把一次 reaction 拆为模型加载、prefill、decode、JSON 校验四段，分别记录 cold/warm。短 JSON 和短台词应将输出上限控制在 64–128 tokens；输入 schema、候选列表和状态摘要也会影响 prefill。建议验收：GPU/Vulkan 热请求完整响应 P95 ≤1 s；CPU-only 可放宽到 ≤2 s；任何超时都立即显示固定台词。上述数字必须由目标机器测试后再写入产品承诺。

### 结构化输出/工具调用可靠性

- Ollama JSON Schema 与 llama.cpp GBNF 能降低语法错误，但不能使 0.6B 模型具备可靠的工具调用规划能力；“JSON 合法”与“选项正确”是两项独立指标。[Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs)；[llama.cpp GBNF](https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md)
- reaction schema 应只允许枚举 `reaction_id`、`tone`、短 `line` 和可选置信度；不允许模型返回工具名、文件路径、网络 URL、任意动作参数或代码。
- Rust 必须拒绝未知字段、超长文本、非法枚举和不在当前候选集合中的 ID；解析失败、服务断开、模型未加载和 deadline 到期统一走确定性 fallback。不要把普通聊天文本再次解析成命令。

### 进程隔离

Ollama/llama-server/OpenVINO sidecar 都应视为不可信的可失败本机服务：Core 通过 loopback HTTP 调用，设置连接/读取/总 deadline，限制响应体大小，单飞（single-flight）reaction 请求，记录退出码和回退原因。Tauri Body 不直接接触 runtime，也不执行模型输出；runtime 进程不应拥有不必要的网络或文件权限。不要让模型加载或卸载阻塞 `reduce`/tick；聊天与 reaction 最好使用独立请求队列，避免长回复饿死低频反应。

## 5. 推荐落地顺序

1. **PoC 默认：Ollama + Qwen3 0.6B（non-thinking）**；以 Gemma 3 1B 和 SmolLM2 1.7B 作基线。固定 100 条中/日/英 ReactionContext，测 schema 成功率、候选命中率、P50/P95、冷/热峰值内存。
2. 若 Arc 驱动表现稳定，**同一模型用 llama.cpp Vulkan** 对比 Ollama；保留 CPU fallback。若无稳定收益，不增加第二 runtime 的发布复杂度。
3. 只有当功耗、NPU 或单进程部署成为明确需求时，才做 **OpenVINO/ONNX Runtime GenAI** benchmark；记录转换限制、EP fallback 和 Rust/sidecar 成本。
4. 发布时锁定模型版本/digest、量化、prompt/schema 和运行时版本；任何新模型先通过回归集。Core 的健康、服从、合法动作、工具权限和安全阈值继续由规则实现。

## 参考链接

- [Ollama Windows](https://docs.ollama.com/windows)
- [Ollama API introduction](https://docs.ollama.com/api/introduction)
- [Ollama Generate API](https://docs.ollama.com/api/generate)
- [Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)
- [llama.cpp Vulkan build](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#vulkan)
- [llama.cpp GBNF](https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md)
- [ONNX Runtime GenAI](https://onnxruntime.ai/docs/genai/)
- [ONNX Runtime DirectML EP](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)
- [OpenVINO GenAI guide](https://docs.openvino.ai/2025/learn-openvino/llm_inference_guide/genai-guide.html)
- [Qwen3-0.6B](https://huggingface.co/Qwen/Qwen3-0.6B)
- [Gemma 3 1B](https://huggingface.co/google/gemma-3-1b-it)
- [SmolLM2 1.7B](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct)
- [Llama 3.2 1B](https://huggingface.co/meta-llama/Llama-3.2-1B-Instruct)
- [Phi-4-mini](https://huggingface.co/microsoft/Phi-4-mini-instruct)
