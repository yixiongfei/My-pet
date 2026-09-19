# 04 · Body：动画规则

代码：`src/body/AnimationPlayer.ts`（播放器）· `interaction.ts`（交互状态机）· `animationPool.ts`（动画池）· `petState.ts`（活动 → 动画映射）· `PetCanvas.tsx` / `Bubble.tsx` / `Countdown.tsx`（布局）· `scripts/build-assets.mjs`（资产）。
**Body 不决定她在做什么**，只决定「看起来在干什么」——活动、心情、食物都由 Core 的 `pet:state` 推来。

## 1. 原版的动画规则（我们完整承接）

资产来自 VPet-Simulator 的 `mod/0000_core/pet/vup/`：当前本机 6875 帧 1000×1000 PNG，按目录组织。原版 `GraphInfo.cs` **从路径推断**每段动画的四个属性：

```
路径按 \ 和 _ 拆 token，依次吃掉：
  ① 心情   Happy | Nomal | PoorCondition | Ill        （没有 → Nomal）
  ② 类型   default | say | touch_head | touch_body | idel | sleep | work | move | raised_* | sidehide_* | switch_* …（GraphType）
  ③ 段落   A / Start → A_Start    B / Loop → B_Loop    C / End → C_End    Single（没有 → Single）
  ④ 名字   剩下 token 的最后一个（去掉数字 / ~ 变体后缀）；没有 → 类型名
帧文件：<前缀>_<序号>_<时长ms>.png        原版同目录 info.lps 可覆盖任何字段
```

这套推断现在只在 `pnpm convert:pet` 迁移时执行一次。迁移结果写进
`assets-src/pet/vup.json`，每段动画的 `source / type / name / mood / segment / layer`
以及每帧文件和时长都显式保存；日常构建不再靠路径重新推断。

**动作类型 AnimatType**（原版 `enum AnimatType { Single, A_Start, B_Loop, C_End }`）：

| 类型 | 含义 | 用途 |
|---|---|---|
| Single | 单一动作 | 完整的独立动画，不分段 |
| A_Start | 开始动作 | 入场 / 准备 |
| B_Loop | 循环动作 | 主体，可无限重复，同名可有多个变体随机挑 |
| C_End | 结束动作 | 退场 / 收尾 |

**ABC 三段式**：`play(type, name, mood)` → A（若有）→ B（循环到 `stop()`）→ C（若有）→ `onIdle` 回到当前活动。B 段每一轮从同名变体里随机挑，所以「睡觉」不是一帧一帧一模一样地循环。

**四套心情目录**：每种类型下按 `Happy / Nomal / PoorCondition / Ill` 分目录，对应 Core 的 `mood`（[03 §3](03-core.md)：健康 < 25 → Ill；体力 < 20 或心情 < 40 → PoorCondition；心情 ≥ 70 → Happy）。请求的心情没有时按 `happy → nomal → poorcondition → ill` 降级找最近的（原版 `FindGraphs` 同样如此）。

**夹心动画**（吃 / 喝 / 收礼 / 吃药）：`back_lay`（宠物本体）→ 食物精灵 → `front_lay`（手）。`vup.json.layered[].food` 保存逐段 `{ms,x,y,width,rotate,opacity}` 轨迹（迁自原版 `info.lps` 的 `FoodAnimation`）；前后两层帧数不同但总时长相同，播放器用**一个时钟**反查各层帧号，不会漂移。手在食物前面——层序错了就穿帮。

## 2. 播放器 `AnimationPlayer`

- Canvas 2D，rAF + 累计时间按每帧自带的 ms 推进（不用 setInterval，后台节流不漂）。
- `createImageBitmap` 按 clip 预解码，LRU 缓存 192 MB。
- `play()` 三段式；`playOnce()` 只播一轮（摸头、拆礼物）；`playStep()` 由外部编排单段（移动、侧挂）。
- **目标不存在**时延迟 400 ms 再回调 `onIdle`，连续 8 次找不到就放弃——否则微任务里 play → 没有 → onIdle → play 会把渲染线程锁死（v0.1.0 的送礼卡死就是这个）。`toActivity` 也先查 `hasClip` 再下目标。
- 每画完一帧把画面缩到 48×48 读回 alpha，打包成掩码推给 Core 做穿透命中。

## 3. 状态 → 动画：行为树与动画池

Core 定活动，Body 按这棵树挑画面：

```
萝莉斯
├─ 紧急状态（Core：生理急需压过一切）
│   ├─ 体力耗尽 → Sleep     ├─ 严重饥饿 → Eat     ├─ 严重口渴 → Drink     └─ 病重 → 正事全停
├─ 用户交互（Body：高优先级、临时，播完回到当前计划）
│   ├─ Gift → 拆礼物一次     ├─ Medicine → 吃药（夹心）     ├─ Touch → 摸头 / 摸身     └─ Drag → 提起 / 挣扎 / 落地 / 侧挂
├─ 当前计划（Core 定活动，Body 在池里轮换，animationPool.ts）
│   ├─ Work  → 写文案 8–15 min · 清屏 5–10 · 直播 10–20 · 烧烤 6–12 · 修屏幕 6–12
│   ├─ Study → 看书 8–15 · 写字 6–12 · 研究 6–12 · 画画 8–15
│   ├─ Play  → 打游戏 6–12 · 删错误 5–10 · 跳绳 3–6 · 玩水 5–10 · 网球 / 舞蹈 4–8
│   └─ Sleep → 入睡（A）· 熟睡（B 随机变体）· 醒来（C）
│   └─ 跳舞（你在放歌，Core 的 dance）→ ACTION_POOLS.music：Music 3–6 · Music2 2–4 · cosplay 2–4 · ohhhh 1–3
│                                       歌到高潮（`pet:music` climax）→ 换成 ohhhh，过去了换回
└─ 无任务（Idle）→ 发呆（default）· StateONE → StateTWO 成对待机 · 四处看 / 小动作（idel，15–40 s 一次）
                   · 日常（Relax/MI · MU · BDay，三成）· 飞吻（WORK/kiss，全身状态都很高时两成）· 走路 / 爬墙 / 坠落（pet_motion）
```

- **按 Core 指名动画开的池**（`ACTION_POOLS[action.graph]`）优先于按活动的池：「跟着歌跳舞」是 playing，但只在舞蹈动画里轮换。
- **吃饭**：每顿两成概率是 `common/eatmcdonald`（普通动画，汉堡画在里面），其余走夹心动画；进入吃饭时掷一次、整顿不变；吃药不算。

- **每段池动画有自己的驻留时长**（表里的分钟区间随机抽）。驻留没到**不因轮换而换**；到了就在同池里随机换一个**不同的**。刚进活动优先播 Core 指名的（`action.graph`）。
- 允许打断的只有三种：用户交互、生理急需（Core 换了活动）、活动自然结束（Core 换了活动）。被交互打断后回来**接着播原来那段**，不重抽。
- Core 的活动、时长、数值一概不受池影响——池只管画面。
- 不在池里的活动走 `CLIP_FOR` 直接映射：`idle → default`，`sleeping → sleep`，`eating / drinking → common/eat · drink`（夹心），`gift → default`（拆礼物那一遍由 `pet:gift` 事件触发，待机不循环）。

## 4. 交互

| 输入 | 表现 |
|---|---|
| 摸头 / 摸身（`vup.json.profile` 的 touchhead / touchbody 区域） | `touch_head` / `touch_body` 三段式一遍，回到当前活动 |
| 按在脸上拖（`pinch` 区域） | 捏脸 `common/pinch`：A 捏住 → B 循环到松手 → C 放开；窗口不动，算一次摸头 |
| 按住拖动 | 只有从当前心情的 `touchraised` 区域起手才会提起；左右半区分别选择两套动作。先播放一次 `raised_dynamic`，随后进入 `raised_static`；窗口在 Rust 侧跟随物理光标。快速松手只继承鼠标水平速度并受重力下落，慢速松手直接归位；Rust 确认落地后 Body 才播放收尾，避免空中提前恢复待机 |
| 松手出屏 | 不到侧挂份上的一律**弹回**当前显示器（头顶区可以在屏幕外，身体不行） |
| 拖过左 / 右边 50 逻辑像素 | 左侧使用 `SideHide_Left_Main`，右侧使用 `SideHide_Right_Main` 播放 A→B；鼠标悬停分别切换 `SideHide_Left_Rise` / `SideHide_Right_Rise` 的 A→B，离开时播放 Rise 的 C 段后回到 Main；按下播 Main C 并完整回到屏幕 |
| 真正空闲 | 每 8–20 秒触发一次小动作；自主移动概率 35%，其余在伸懒腰 / 喝茶 / 庆祝 / 待机姿态间选择；MI / MU 会循环 8–12 秒后再收尾；蹲下动作按资源标注的 125ms 帧时长平滑起立；移动仍按原版 16 条 `move` 规则走路 / 爬行 / 爬墙 / 顶部移动 / 坠落，位移在 Core 50 ms 轮询里按规则 Interval 推，每步做边界检查 |
| 特殊日期 | 启动完成后按本机日期每天检查一次：1 月 1 日播放 `startup/newyear`，2 月 14 日播放飞吻，6 月 7 日祝福用户生日，8 月 14 日庆祝角色生日，12 月 25 日播放节日庆祝；生日和节日使用 `bday`，同时通过动作匹配的开心 / 害羞语音说祝福；两个生日庆祝动作结束后会播放登记在食物目录中的 `birthday-cake`，明确拿出生日蛋糕，不占日常空闲动作概率 |
| 说话（TTS 在放） | 按这句话的类别循环 `say/*`：自言自语 / 动作台词 → `self`；日程提醒、状态差时 → `serious`；开心时的对话 → `shining`；道谢 / 害羞字眼 → `shy`；换句换风格才换动画；一次性动画播完再接；提起时不说话 |
| 启动 / 托盘退出 | `startup` 一次；退出先播 `shutdown`，完成后才结束进程（8 s 超时兜底） |
| 等模型首字 | `common/think` A → B 循环；首字、取消或失败时播 C 回当前活动 |
| 升级 / 心情变化 | `common/levelup`；心情上升 `switch_up`、下降 `switch_down` |
| 进入吃 / 喝 | 先播 `switch_hunger` / `switch_thirsty`，再接夹心动画；吃药不误播饥饿 |
| 单击 / `Alt+V` | 打开对话窗口 |

**穿透**：不用原版的矩形，用当前帧 alpha。Core 每 16 ms 读光标、查掩码，只在结果变化时切 `ignore_cursor_events`；掩码没到、位置读不到一律**不穿透**（穿透错了她就再也点不着）。交互期间 `set_hit_test_pinned(true)` 钉住。头顶区永远穿透。

## 5. 布局：头顶区、气泡、倒计时

- pet 窗口高 = 宽 × **1.6**：下面的正方形是立绘（500×500 逻辑参考系，掩码 / 触摸区 / 拖拽锚点只按它算），上面 0.6 倍边长是**头顶区**。比例 `HEAD_ROOM` 在 `lib.rs` 和 `PetCanvas.tsx` 各一份，成对改。
- **气泡**：尾巴指向头，按头顶区高度决定最多几行（≤ 7），超出打「…」；流式输出只显示尾部 120 字；念完之前不收。头顶区穿透，所以气泡上没有按钮。
- **倒计时环**：番茄钟 / 专注段挂在头顶；气泡在的时候缩成小药丸让位。
- 显示大小 200–800 px 可调，多屏与 DPI 缩放在 Rust 侧换算。

## 6. 资产管线 `scripts/build-assets.mjs`

```
一次迁移（pnpm convert:pet）
  vup.lps + vup/**/info.lps + 目录路径
    └─ GraphInfo 规则只推断一次
       └─ assets-src/pet/vup.json
          ├─ profile：触摸 / 提起 / work / 16 条 move / 侧挂锚点
          ├─ animations：659 段；type / name / mood / segment / layer
          ├─ frames：6875 个 PNG 文件名与 ms
          └─ layered：12 条前层 / 后层 / 食物轨迹，全部可用（喝水 Happy / PoorCondition 复用 Nomal 手部前层）

一次迁移（pnpm convert:food）
  food/*.lps ──→ assets-src/food/food.json
                  ├─ gifts 20 · foods 66 · drinks 27 · medicines 10
                  └─ 图片路径 · 营养 · 价格 · 健康 · 描述 · 原始来源

日常构建（pnpm build:assets）
  vup.json + vup/**/*.png ──→ WebP + pet.json + manifest.json
  food.json + image/*.png ──→ manifest.food + Core food-catalog.json（123 项；图转 128 px WebP）
```

`vup.json` 是后续优化动画归类的编辑入口，结构见
`schemas/pet-source-v1.schema.json`；食物结构见 `schemas/food-source-v1.schema.json`。构建会拒绝未知 type / mood / segment、越界相对路径、
空动画以及不存在的帧；重新运行 `convert:pet` 会从原版 LPS 覆盖它，所以手工调整前应确认是否要重迁移。

`assets-src/` 中的 PNG / LPS 与生成的 `public/pet/` 不入库；只提交不含图片的
`assets-src/pet/vup.json`、`assets-src/food/food.json` 映射与 Schema，让后续动画 / 食物归类优化能正常 review、回滚和发布。
- 角色对话气泡的打字机显示采用 14ms/字；当显示落后超过 24 字时，以 6ms/次、每次最多 5 字追赶，避免流式回复已经收到但画面显示过慢。该优化只影响气泡呈现，不改变 Core 文本、句子切分或 TTS 顺序。
