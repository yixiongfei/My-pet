# JEV / Jev 身份核查（截至 2026-09-19）

## 结论先行

截至 2026-09-19，“JEV”不是一个可以唯一指向单一公开小语言模型的稳定名称。至少有四个应区分的公开指称：

1. **Jev（TypeSafe AI）**：最强、最权威的当前公开指称。TypeSafe 称它为首个 **System One Model**，面向软件内的 typed decision，而不是聊天、语音或自由文本生成。[TypeSafe AI 首页](https://typesafe.ai/)、[官方 Introduction](https://docs.typesafe.ai/introduction.md)
2. **open-jev-deberta-v3-large（Kotoba Labs）**：Hugging Face 上的可下载、开源/Apache-2.0 的独立复现，明确声明“复现 Jev 的形状”，不隶属于 TypeSafe、不使用其数据或代码；它是 DeBERTa-v3-large 编码器 + typed-decision head，不是 TypeSafe 的原始权重。[Hugging Face model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)、[API metadata](https://huggingface.co/api/models/com-kotobalabs/open-jev-deberta-v3-large)、[Kotoba typed-decisions](https://github.com/kotoba-lang/typed-decisions)
3. **jev-lm（y0usaf）**：GitHub 上的实验性 TypeScript 项目，使用 TypeSafe Jev API 的 Choice/Noul 来验证词块、实现 word-level LM；它不是一个本地权重模型，也不是角色/反应模型。[GitHub repository](https://github.com/y0usaf/jev-lm)、[GitHub API metadata](https://api.github.com/repos/y0usaf/jev-lm)
4. **项目内部简称/误称**：若上下文是本地桌宠、角色反应或语音管线，“JEV small model”仍可能只是团队内部对“判断/评价/反应选择器”的简称；公开检索没有证明存在一个名为 JEV 的主流本地角色模型或语音模型。这个结论是“未找到权威证据”的有限负面判断，不是证明不存在私有项目。

因此，在 VPet 语境中最安全的写法是：**“Jev 是 TypeSafe 的决策模型；open-jev 是独立开源复现；若说的是本地反应小模型，JEV 仍需提供具体仓库、模型 ID 或上下文，不能默认等同于其中任何一个。”**

## 官方 TypeSafe Jev：身份与能力

### 公开身份

- TypeSafe AI 官网把 Jev 定义为其首个公开的 System One Model；System One 的定位是“machine-native intelligence”，让软件直接消费 typed decisions，而非让人阅读文本。[官方首页](https://typesafe.ai/)
- 官方文档写明：Jev 接收一个 state 和 typed questions，返回结构化结果；“No text generation, no parsing”。可用原语为 **Choice**（从选项中选择）、**Score**（按有序等级评分）和 **Noul**（真假/yes-no 概率）。[官方 Introduction](https://docs.typesafe.ai/introduction.md)
- TypeSafe 首页明确把产品定位为 “not chat” 和 “decisions, not strings”；页面给出的性能/价格数字是厂商宣传内容，不能当作独立基准结论。[官方首页](https://typesafe.ai/)、[官方博客](https://typesafe.ai/blog/introducing-system-one-models-and-jev)

### 与反应/角色模型的关系

- Jev 的接口在抽象层面适合“从有限反应中选择”“给情绪/意图打分”“判断是否触发某动作”：Choice 可选 reaction_id，Score 可评估情绪/服从度，Noul 可做触发条件。这是**架构上的推论**，不是 TypeSafe 官方把 Jev 宣传为角色模型。[官方 Introduction](https://docs.typesafe.ai/introduction.md)
- Jev 不负责生成角色台词、语音波形、自然语言解释或长期人格记忆；官方模型说明直接强调输出是 typed values/probabilities，不是文本。[官方 Introduction](https://docs.typesafe.ai/introduction.md)
- 对 VPet 的确定性 Core，若采用此类模型，合理边界是让模型做受限的候选选择/评分，仍由 Core 负责状态变更、阈值、随机性和安全约束。这个是工程建议，不是外部事实。

## 公开可下载/可复现的 JEV 相关项目

### `open-jev-deberta-v3-large`

- Hugging Face 条目明确称其为“An open, Jev-shaped typed-decision model”，输入一个 state 和任意多个 typed questions，单次 forward pass 返回每个问题的 calibrated distribution；它不生成文本。[Model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)
- Model card 明确声明：这是独立复现 TypeSafe Jev 的“shape”，**not affiliated with TypeSafe, uses none of their data or code, and its numbers are not comparable to theirs**。[Model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)
- 可验证的实现信息：基于 `microsoft/deberta-v3-large`；Transformers/text-classification 元数据；Apache-2.0；文件包含 `typed_decisions` Python 包、`model.safetensors`、`head.safetensors` 和 `open_jev_config.json`。[HF API metadata](https://huggingface.co/api/models/com-kotobalabs/open-jev-deberta-v3-large)
- 能力/限制（来自模型卡自报）：Choice 最多 255 个选项、Score 2–10 个有序等级、Noul yes/no；总上下文 512 tokens，state 截断至 256；英文；训练数据为 Banking77、SST-5、BoolQ；OOD 指令/选项集性能低于 in-domain，未见过的有序评分等级尤其弱；不能生成文本。[Model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)
- Model card 的实测数字（0.854 in-domain accuracy、0.690 OOD 等）应视为该独立复现的报告，不应转述为 TypeSafe Jev 的性能。[Model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)
- 对本项目的含义：它是目前找到的最接近“可本地运行的 Jev 形状小模型”的公开候选，但底座是约 434M 参数的 DeBERTa-v3-large 权重规模，并且只支持英文/有限领域；不能假定它适合中文/日文桌宠反应，必须做本地数据评测。[HF API metadata](https://huggingface.co/api/models/com-kotobalabs/open-jev-deberta-v3-large)、[Model card limitations](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)

### `y0usaf/jev-lm`

- GitHub 项目描述为“A word-level language model whose output layer is Jev”，通过 TypeSafe API 的 `Choice`/`Noul` 验证词块；仓库 README 明确写明 Jev 本身不发射文本，项目把 tokenizer、sampler、repetition mask、stop rule 等传统 LM 隐含部分手工实现。[Repository](https://github.com/y0usaf/jev-lm)
- README 的 live calls 使用 `api.typesafe.ai/v1/systemone`、模型 `jev-1.13.0`；因此它是一个依赖远程 TypeSafe API 的实验 wrapper，不是 Ollama/llama.cpp 可离线加载的 JEV 权重。[Repository](https://github.com/y0usaf/jev-lm)
- 仓库把自身定位为实验/解释性质的实现，并报告其 229-word vocabulary 的 bits/token 不优于同词表 unigram baseline；README 还明确说“不应作为产品中的 text generator”。[Repository](https://github.com/y0usaf/jev-lm)
- 这与“JEV 是一个可本地部署的角色反应小模型”相冲突：该 repo 的用途是研究 typed decision API 如何组成生成循环，而不是提供人格、台词或语音能力。[Repository](https://github.com/y0usaf/jev-lm)

## 与本地 AI / 小模型生态的对照

- Ollama 的官方 structured outputs 文档展示的是对通用聊天模型施加 JSON Schema/`format` 约束，并要求调用方验证 JSON；这能实现 reaction schema，但不等同于 Jev 的原生 typed-decision architecture。[Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs)
- Qwen3-0.6B 的官方 Hugging Face 卡片把它定义为 0.6B causal language model，支持多语言、对话、角色扮演、agent 能力，并可由 Ollama、llama.cpp 等本地工具部署。[Qwen3-0.6B model card](https://huggingface.co/Qwen/Qwen3-0.6B)、[Ollama Qwen3](https://ollama.com/library/qwen3)
- 由此可区分两种设计：Qwen3/类似 SLM 是“生成文本，再用 JSON/schema 约束”；Jev/open-jev 是“直接对给定问题输出选择/分数/真假分布”。前者天然适合台词和角色对话，后者更适合反应选择器、意图/情绪分类器或路由器。该比较依据上述官方接口描述，属于架构推论。[Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs)、[TypeSafe Introduction](https://docs.typesafe.ai/introduction.md)、[open-jev model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)
- 未找到官方 Ollama library 条目、官方 llama.cpp model card 或官方 Hugging Face model card 表明存在一个名为“JEV”的主流中文/日文角色、reaction、TTS 或 companion 模型；但搜索结果受命名、私有仓库和索引延迟影响，不能据此断言绝对不存在。

## 对既有笔记的核验与修正

既有笔记 `D:\obsidian\Vpet\research_notes\JEV small model reactions\jev_identity.md` 的核心判断——TypeSafe Jev 是决策模型而非语音/角色生成模型、在本地 companion 场景中可能只是内部 shorthand、没有证据证明它是主流 voice model——仍然成立。[TypeSafe Introduction](https://docs.typesafe.ai/introduction.md)

但其中“唯一找到的公开 JEV 是 TypeSafe Jev”“没有 authoritative public model-card 或 official repo naming a model JEV”已被 2026-09-18/19 的新公开资料推翻或需要改写：

- 已发现公开 Hugging Face `com-kotobalabs/open-jev-deberta-v3-large`，它明确使用 Jev 名称但声明是独立复现。[Hugging Face model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)
- 已发现 GitHub `y0usaf/jev-lm`，它是使用 Jev API 的实验性 word-level LM wrapper。[GitHub repository](https://github.com/y0usaf/jev-lm)
- 因而“没有唯一可识别模型”仍是正确的总判断，但理由应从“只有 TypeSafe 一个公开命中”更新为“公开命中至少包括 TypeSafe 原版、open-jev 独立复现和 jev-lm wrapper，彼此不是同一 artifact”。

## 不确定性、检索边界与建议

- **高置信**：在 2026-09-19 的公开记录中，TypeSafe AI 的 Jev 是最权威且最可能被“JEV”指代的产品/模型。[TypeSafe AI](https://typesafe.ai/)、[官方 Introduction](https://docs.typesafe.ai/introduction.md)
- **高置信**：`open-jev-deberta-v3-large` 不是 TypeSafe 原版，而是明确声明独立的开放复现；不能写成“TypeSafe 发布了开源 JEV 权重”。[Model card](https://huggingface.co/com-kotobalabs/open-jev-deberta-v3-large)
- **高置信**：`jev-lm` 不是本地模型权重，也不是 reaction/character model；它依赖 TypeSafe API。[GitHub repository](https://github.com/y0usaf/jev-lm)
- **中置信**：在用户所说的“reaction/character/local AI”语境中，JEV 很可能是把“判断层/反应选择器”当作内部名称，或把 Jev 与普通 SLM 混称。公开来源没有给出该内部含义，因此不能当作事实。
- **关键缺口**：TypeSafe 官方页面说明的是 API/产品能力，没有公开本地权重、Ollama Modelfile、llama.cpp GGUF 或可离线运行说明；因此不能把官方 Jev 当作本机 Core 可直接部署的模型。[TypeSafe docs index](https://docs.typesafe.ai/llms.txt)、[官方 Introduction](https://docs.typesafe.ai/introduction.md)
- 若要在 VPet 中继续确认“JEV”所指对象，应要求提供至少一个可复现标识：完整 Hugging Face model ID、GitHub URL、Ollama tag、API endpoint/model name，或原始对话/文档上下文。没有这些标识时，文档应同时写出三种可能：TypeSafe Jev、open-jev 复现、内部 shorthand。
