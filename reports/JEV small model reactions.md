# JEV 小模型反应方案：让模型建议、让状态机裁决

> 研究截止：2026-09-19（日本时间）。本报告基于 `research_notes/JEV 小模型集成/`、`research_notes/JEV small model reactions/` 和仓库文档；不修改源代码。

## 结论先行：可以导入“小模型”，但不能把它当状态机

**可以把一个本机小语言模型接入 VPet，让她在事件发生时选择更合适的台词、情绪标签或候选动作；不建议让模型直接修改饥饿、健康、服从、关系值，也不建议让模型直接决定任意动作。** 最稳妥的边界是：Rust Core 继续用纯函数和显式时间/随机源维护确定性状态；模型作为异步、可失败的“反应建议器”；Core 只接受经过 schema、allowlist、权限和状态条件校验的结果。模型超时、不可用、输出非法或置信度不足时，系统必须自动回到规则反应，用户不应看到错误。

“JEV”目前不能唯一识别为一个可下载、可离线导入 VPet 的公开模型。公开资料中最接近的 **Jev** 是 TypeSafe AI 面向结构化判断的产品，并非中文陪伴模型或 TTS 模型（外部事实：[TypeSafe AI](https://typesafe.ai/)、[Jev 文档](https://docs.typesafe.ai/introduction.md)）。研究笔记还记录了 `open-jev-deberta-v3-large` 等同名线索，但它们不能据此被认定为适合生成角色反应的生成式小模型；在没有官方权重、许可证、推理格式和本地接口之前，不应把“JEV”写死成依赖。推荐先用已验证的 Qwen3/Gemma/SmolLM2 等小模型验证架构，再根据用户提供的确切模型链接或权重决定是否替换。

## JEV 的身份必须先降级为“待确认依赖”

**外部事实：** TypeSafe 的 Jev 强调 typed decisions、Choice/Score/Noul 等结构化决策原语，目标是让软件消费判断结果，而不是生成台词或语音（[TypeSafe AI](https://typesafe.ai/)，[Jev introduction](https://docs.typesafe.ai/introduction.md)）。这使它在“反应选择器”的概念上有启发，但不等于存在可下载的 `jev:0.x` Ollama 模型。Ollama 的公开模型库列出的是 Qwen、Gemma、Llama、SmolLM 等模型家族，而不是一个可确认的 JEV 家族（[Ollama Library](https://ollama.com/library)）。

**仓库观察：** 两组研究笔记都把 JEV 身份标为不确定，并建议将它视为候选“判断/反应选择层”，而不是语音模型。当前 VPet 已经把本机 Ollama、Qwen3-TTS 和确定性 Rust Core 分开；语音引擎只能把已确定的台词变成声音，不能反过来决定状态。因而本报告的建议不依赖 JEV：JEV 若最终确认是可导出为 GGUF 的生成模型，可接入同一个 adapter；若是分类/判别模型，则只能作为标签或评分器；若只有云端 API 或不明许可证，则不纳入发布版。

## 本机可行路线：先选成熟运行时，再验证 JEV

### Ollama 是第一阶段默认方案

Ollama 在 Windows 提供本机服务和模型管理，支持结构化输出约束；其文档明确给出 JSON schema 用法，适合让模型返回 `reaction_kind`、`line_key`、`emotion` 等有限字段（[Ollama Windows](https://docs.ollama.com/windows)，[Structured outputs](https://docs.ollama.com/capabilities/structured-outputs)）。仓库已经使用本机 Ollama，因此接入成本、启动脚本和故障处理最低。候选基线可用 Qwen3 0.6B、Gemma 3 1B 或 SmolLM2 1.7B；应以实际机器的首 token 延迟和中文输出质量为准，而不是只看参数量（[Qwen3 0.6B](https://huggingface.co/Qwen/Qwen3-0.6B)，[Gemma 3 1B](https://huggingface.co/google/gemma-3-1b-it)，[SmolLM2](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct)）。

Qwen3 需要关闭 thinking 或把思考预算限制为零，避免反应延迟和多余文本；prompt 中只允许输出 schema，temperature 设为 0 或尽可能低，并在客户端再次解析。即使 temperature 为 0，也不能把服务返回当作数学意义上的确定性：模型版本、采样实现和硬件仍可能变化，所以“模型建议”必须是可丢弃的非权威输入。

### llama.cpp + GGUF 是发布版的第二路线

llama.cpp 可直接运行 GGUF 模型并提供本机 HTTP server，适合希望减少 Ollama 依赖、控制线程/GPU 层数或随桌面应用打包的场景（[llama.cpp GitHub](https://github.com/ggml-org/llama.cpp)，[GGUF 说明](https://github.com/ggerganov/ggml/blob/master/docs/gguf.md)）。代价是模型下载、DLL、Vulkan/CUDA/CPU 参数、进程生命周期和升级都要由项目维护。建议先保持 Ollama adapter 的协议，再增加 llama.cpp adapter，不要在第一阶段同时引入两套运行时。

### ONNX Runtime GenAI、OpenVINO 和直接嵌入暂不作为默认

ONNX Runtime GenAI 可用于生成式模型和不同 Execution Provider（[Microsoft ONNX Runtime GenAI](https://github.com/microsoft/onnxruntime-genai)）；OpenVINO 更适合 Intel CPU/GPU/NPU 优化（[OpenVINO GenAI](https://github.com/openvinotoolkit/openvino.genai)）。但模型转换、Rust FFI/sidecar、算子兼容和 Windows 打包会显著增加风险。只有当发布目标明确要求无独立本机服务、或硬件优化收益已用基准测试证明，才应进入第二阶段。

### 能否“导入 JEV”取决于四个可验收条件

在宣布可导入前，必须拿到：**权重或可访问服务；明确模型格式（GGUF、Safetensors、ONNX 等）；允许本地推理的许可证；可复现实验的输入输出协议。** 若 JEV 只有 TypeSafe 的远程服务，则它不符合本项目“只接本机地址、不联网、不上传”的约束；若它是 DeBERTa 类判别模型，则不能直接生成台词；若没有模型卡和许可证，不能放进发行包。模型身份未确认前，用 Qwen3/Gemma/SmolLM2 做 adapter 验证是可行且可回退的方案。

## 推荐架构：Core 裁决，Brain 组织，模型只提议

一次反应建议的路径应是：

`用户事件/定时 tick → Rust Core reduce → ReactionRequest 快照 → 本机模型 adapter → JSON 校验 → policy gate → Core request_action（仅在允许时）→ Core 产出事件 → React Body/TTS 展示`

**第一层是状态真相。** `reduce` 仍然接收外部传入的时间和随机数；模型不得直接写 state。健康阈值、疲劳、服从判定、冷却、可用动作和资源消耗由 Core 计算。模型输入只读快照，例如事件类型、当前 mood、最近事件摘要、可用 reaction key 和剩余冷却时间；不要把整个内部状态或不可公开记忆原样交给模型。

**第二层是受限输出。** 建议第一版 schema：

```json
{
  "kind": "say",
  "line_key": "tease_01",
  "emotion": "shy",
  "animation": "idle",
  "confidence": 0.78,
  "reason_code": "user_greeting"
}
```

`kind`、`line_key`、`emotion`、`animation` 都必须是编译期或配置文件 allowlist；模型不能返回任意文本作为动作名，不能返回数值增量，不能直接请求外部命令。`line_key` 只引用本地台词表，台词插值仍由 Brain/Core 控制。对用户意志产生的动作，模型只能产生候选 `action_id`，最终必须调用已有 `request_action` 入口接受服从判定；不能通过“聊天命令”绕过规则。

**第三层是 policy gate。** 校验顺序建议为：解析 JSON → schema 类型/范围 → allowlist → 当前状态权限 → 冷却/去重 → 置信度阈值 → 事件版本是否仍然有效。模型响应带 `snapshot_id` 和 `state_revision`；若响应回来时版本已改变，就丢弃，不把过期建议应用到新状态。模型调用异步运行，不能阻塞 tick 和 UI。

**第四层是展示。** React Body 只消费 Core 事件，不实现业务判断；动画 key、气泡、台词和语音都由事件驱动。这个边界符合仓库对 Body/Core/Brain 的分层约定（仓库观察：`docs/02-components.md`、`docs/04-body-animation.md`、`docs/05-brain.md`）。

## 失败与回退必须和成功路径一样明确

模型不可达、超时、HTTP 错误、JSON 解析失败、schema 不符、低置信度、版本过期或命中安全策略时，统一进入 `reaction_fallback`。fallback 只使用已测试的规则候选：按事件类型、状态阈值、冷却和 Core 的随机源选择固定台词/动画。**fallback 不能再次调用模型，也不能改变数值。** 建议首版超时 300–800 ms；超时后立即展示规则反应，模型结果即使稍后回来也因 `state_revision` 不匹配而丢弃。

模型结果应写入可选的诊断日志：模型 ID、运行时版本、输入快照哈希、输出、耗时、拒绝原因和 fallback 原因；不要记录完整私人对话或健康隐私。发布版默认关闭详细 prompt 日志。为提高可重复性，可按 `event_id + state_revision` 缓存建议，但缓存也必须经过同一 policy gate；不得把缓存当状态。

## 风险与验收标准

主要风险包括：模型幻觉出不存在的动作；提示注入让用户意志绕过服从规则；模型延迟造成反应顺序错乱；不同模型版本导致台词漂移；显存/内存不足拖慢桌宠；模型许可证不允许再分发；JEV 身份误判导致错误依赖。安全边界是“模型输出不可信、Core 输入必须可验证”。本机运行仍要限制为 `chat::local_endpoint`，不增加联网上传路径。

测试应覆盖：固定快照下 JSON 合法/非法样例；所有未知枚举均 fallback；低健康时模型建议危险动作被拒绝；用户命令只能经 `request_action`；超时与服务关闭不阻塞 tick；旧 `state_revision` 被丢弃；相同 `event_id` 重放不会重复扣数值；Ollama 未安装时桌宠仍能完整运行规则反应。`pnpm check` 是提交前总验证入口；模型专项测试应使用假 adapter 和固定响应，不依赖真实 Ollama。

## 按仓库路径的实施路线

**阶段一：只做可观测的 `say` 建议。** 在 `apps/desktop/src-tauri` 增加模型 adapter/超时/解析边界，在 Core 侧只消费 `line_key`；先用 Qwen3 0.6B 或 Gemma 3 1B。同步在 `docs/02-components.md` 记录模块和事件，在 `docs/05-brain.md` 记录 prompt、意图与台词边界，在 `docs/06-roadmap.md` 标注进度，在 `CHANGELOG.md` 的“未发布”说明“为何采用可回退建议层”。

**阶段二：固定候选的情绪与动画。** 在 `docs/04-body-animation.md` 和 `docs/07-animations.md` 登记 allowlist、动画 key 和气泡行为；Core 仍决定是否允许展示，Body 不新增业务逻辑。加入 snapshot/version、缓存和超时测试。

**阶段三：极窄的动作建议。** 只允许模型返回预注册 `action_id`，在 `docs/03-core.md` 记录健康阈值、服从判定、冷却和拒绝原因；所有用户意志仍走 `request_action`。这一步完成并通过回放测试后，才评估是否替换或加入 JEV。

**阶段四：运行时与模型替换。** 先保持 Ollama；若发布体积、启动或可控性成为瓶颈，再实现 llama.cpp/GGUF adapter。JEV 只有在权重、格式、许可证、中文质量、延迟和结构化输出测试全部通过后才成为可选 profile，不能成为 Core 的硬依赖。用户可感知的差异同步写入 `README.md`“和她相处”。

## 结论

答案是“能接入小模型，但不应把未经确认的 JEV 直接导入并交给它优化状态机”。当前证据不足以确认 JEV 是一个公开、可本机部署的反应模型；最稳妥的工程判断是把它当待确认的候选适配器。对 VPet，真正可行的优化不是让模型接管状态，而是让成熟的小模型在确定性状态机给出的边界内选择台词和表现，再由 Core 验证、裁决并在失败时回退。这样既能获得更自然的反应，也保留离线、可测试、可回放和不会因模型波动破坏数值逻辑的特性。

### 主要来源

- [TypeSafe AI](https://typesafe.ai/)；[Jev introduction](https://docs.typesafe.ai/introduction.md)
- [Ollama Library](https://ollama.com/library)；[Windows](https://docs.ollama.com/windows)；[Structured outputs](https://docs.ollama.com/capabilities/structured-outputs)
- [Qwen3 0.6B](https://huggingface.co/Qwen/Qwen3-0.6B)；[Gemma 3 1B](https://huggingface.co/google/gemma-3-1b-it)；[SmolLM2](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)；[GGUF](https://github.com/ggerganov/ggml/blob/master/docs/gguf.md)
- [ONNX Runtime GenAI](https://github.com/microsoft/onnxruntime-genai)；[OpenVINO GenAI](https://github.com/openvinotoolkit/openvino.genai)
