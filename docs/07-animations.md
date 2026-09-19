# 07 · 动画实装清单

原版 `vup` 的 87 组动画（`assets-src/pet/vup.json`，659 段 / 6875 帧）哪些接进了行为、怎么触发、哪些还躺着。
✅ 已接 · 🚧 部分 · ⬜ 没接。触发逻辑在 [04](04-body-animation.md)，数值在 [03](03-core.md)。

## 1. 生活（Core 的活动 → Body 动画池）

| 资源 | type / name | 触发 | 状态 |
|---|---|---|---|
| WORK/workone 写文案 · workclean 清屏 · worktwo 直播 · grilledsausage 烧烤 · fixmenu 修屏幕 | `work/*` | 工作（`working` 池，按驻留轮换） | ✅ |
| WORK/study 看书 · calligraphy 写字 · studytwo 研究 · studypaint 画画 | `work/*` | 学习（`studying` 池） | ✅ |
| WORK/playone 打游戏 · removeobject 删错误 · ropeskipping 跳绳 · playwater 玩水 | `work/*` | 玩（`playing` 池） | ✅ |
| Relax/Tennis 网球 | `common/tennis` | 玩（`playing` 池） | ✅ |
| Music · Music2 · saraburate/cosplay · saraburate/ohhhh | `common/music` `music2` `cosplay` `ohhhh` | 你在放歌 → Core `dance` → `ACTION_POOLS.music` 轮换；歌到高潮固定换 `ohhhh` | ✅ |
| Sleep | `sleep` | 睡觉（A 入睡 · B 熟睡随机变体 · C 醒来） | ✅ |
| Eat（夹心 back / front lay） | `common/eat` 夹心 | 吃饭：Core 挑的食物做精灵 | ✅ |
| Eat/EatMcDonald | `common/eatmcdonald` | 每顿饭两成概率（普通动画，不用夹心） | ✅ |
| Drink（夹心） | `common/drink` 夹心 | 喝水（Happy / PoorCondition 复用 Nomal 前层） | ✅ |
| Gift（夹心） | `common/gift` 夹心 | 收礼物：`pet:gift` 播一遍 | ✅ |
| Default | `default` | 发呆 / 收礼待机的底图 | ✅ |
| WORK/reading | `work/reading` | — | ⬜ 可以进学习池（和 study 很像，先没放） |
| WORK/b4 | `work/b4`（single，happy / nomal） | — | ⬜ 不知道是什么场景，没接 |

## 2. 空闲小动作（真正闲着时 15–40 s 一次）

| 资源 | type / name | 触发 | 状态 |
|---|---|---|---|
| IDEL/aside · boring · bubbles · squat · yawning · meow · meowlook · amusement · like520 | `idel/*` | 按心情随机挑一个 | ✅ |
| State（StateONE → StateTWO） | `stateone` / `statetwo` | 三成五概率成对播 | ✅ |
| Relax/MI · Relax/MU · BDay | `common/mi` `mu` `bday` | 三成概率 | ✅ |
| WORK/kiss 飞吻 | `work/kiss` | 体力 ≥ 80、心情 ≥ 80、饱腹 / 口渴 ≥ 70、健康 ≥ 80、好感 ≥ 60 全满足时两成 | ✅ |
| MOVE（walk / crawl / climb / fall 各方向） | `move/*` | 四成概率按原版 16 条规则走路 / 爬行 / 爬墙 / 坠落 | ✅ |
| SideHide_Left_Main · SideHide_Left_Rise · SideHide_Right_Main · SideHide_Right_Rise | `sidehide_left_main` / `sidehide_left_rise` / `sidehide_right_main` / `sidehide_right_rise` | 拖过对应屏幕边缘侧挂；Main 播放侧挂循环，hover 切换 Rise 探头，离开后回 Main | ✅ |

## 3. 交互

| 资源 | type / name | 触发 | 状态 |
|---|---|---|---|
| Touch_Head | `touch_head` | 短按头 | ✅ |
| Touch_Body（tb1 · tb2 · turn · showopai · ill 版） | `touch_body/*` | 短按身体，按心情随机挑 | ✅ |
| Raise（左右半区选择两套：`raised_dynamic` 一次挣扎 → `raised_static` 静止） | `raised_*` | 长按 / 拖动提起 | ✅ |
| Pinch | `common/pinch` | 按在脸上（`pinch` 区域）拖动：A 捏住 → B 循环 → 松手 C | ✅ |

## 4. 说话与过场

| 资源 | type / name | 触发 | 状态 |
|---|---|---|---|
| Say/Self | `say/self` | 自言自语、动作台词、心情一般时的普通对话 | ✅ |
| Say/Serious | `say/serious` | 日程提醒、状态差时的回复 | ✅ |
| Say/Shining | `say/shining` | 开心时的对话 | ✅ |
| Say/Shy | `say/shy` | 收礼 / 吃药的道谢；对话里有「谢谢 / 喜欢你 / 抱 / 脸红」 | ✅ |
| Think | `common/think` | 等模型首字 | ✅ |
| StartUP | `startup/startup` | 程序启动 | ✅ |
| StartUP/newyear | `startup/newyear` | 每年 1 月 1 日启动完成后 | ✅ |
| Shutdown | `shutdown` | 托盘退出，播完再结束进程 | ✅ |
| LevelUP | `common/levelup` | 升级 | ✅ |
| Switch up / down | `switch_up` / `switch_down` | 心情升 / 降一档 | ✅ |
| Switch hunger / thirsty | `switch_hunger` / `switch_thirsty` | 进入吃 / 喝之前 | ✅ |

## 5. 节日与生日

- 1 月 1 日播放 `startup/newyear` 并说新年祝福。
- 2 月 14 日优先播放 `WORK/kiss`，说温柔的节日祝福。
- 6 月 7 日祝福用户生日；8 月 14 日庆祝角色生日；两个生日动作结束后播放 `birthday-cake` 食物夹心动画；12 月 25 日播放 `common/bday` 作为节日庆祝。
- 每次启动每天最多触发一次；本机日期不在列表中时不插入特殊动作。

## 6. 还没接的（2 组）

- `work/reading`：适合进学习池，等看过内容再定驻留。
- `work/b4`：两张 single，不知道原版在哪用；先留着。

其余 85 组全部在用。哪天想调概率：`interaction.ts` 顶部的 `RELAX_CHANCE` / `KISS_CHANCE` / `MCDONALD_CHANCE` / `STATE_IDLE_CHANCE` / `MOVE_CHANCE`，池子的驻留在 `animationPool.ts`。
