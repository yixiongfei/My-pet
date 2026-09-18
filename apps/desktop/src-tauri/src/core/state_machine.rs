//! 宠物状态机（docs/03 §1–3）。
//!
//! 三层分工，谁都不越界：
//!
//! ```text
//!   状态机（本文件）   状态与合法转移、数值不变量、什么时候该换一件事做
//!        ↑ 仲裁
//!   驱力（decide）     生理急需 > 作息到点 > 心情/经济需求 > 兜底
//!        ↑ 查
//!   数据表（actions.toml）  每件事的消耗 / 收益 / 门槛 / 冷却 / 时段
//! ```
//!
//! `reduce` 是纯函数：不读时钟、不做 IO。时间和「现在几点」都从外面以
//! `Event::Tick { minutes, hour }` 喂进来——所以「凌晨三点她在干嘛」「饭点会不会吃饭」
//! 这类行为可以在单元测试里瞬间验证，不用真等到半夜。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::actions::{ActionDef, Catalog};
use super::bias::{Biases, SPILL, SUPPRESS};
use super::food::{FoodShelf, Need};
use super::obey::{judge, Refusal, Verdict, PRESSURE_HURT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    Idle,
    Working,
    Break,
    Studying,
    Sleeping,
    Playing,
    Eating,
    Drinking,
    /// 收礼物。和吃喝一样是过场，不该被打断
    Gift,
}

impl Activity {
    /// 过场动画，不该被新的决策打断（正在把饭往嘴里送，不能突然去上班）
    pub fn is_transient(self) -> bool {
        matches!(self, Activity::Eating | Activity::Drinking | Activity::Gift)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Happy,
    Nomal,
    PoorCondition,
    Ill,
}

/// 她买下的那一样东西。Body 照这个 id 渲染精灵，不再自己随便抓一个
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoodRef {
    pub id: String,
    pub name: String,
}

/// 正在做的事。Body 用 `graph` 挑动画，也能直接显示「正在：文案」
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRef {
    pub id: String,
    pub name: String,
    pub graph: String,
    /// 为什么选了它。让数值变动有迹可循
    pub reason: String,
    /// 吃 / 喝 时她买的那样东西
    pub food: Option<FoodRef>,
}

/// 发给 Body 的 `pet:state` 载荷，字段与 packages/shared/src/pet.ts 的 zod schema 一一对应
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetState {
    pub activity: Activity,
    pub mood: Mood,
    pub strength: f32,
    pub feeling: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub money: f32,
    pub exp: f32,
    pub level: u32,
    /// 好感度 0–100。**慢变量**：天级才看得出变化，和分钟级的 `feeling` 分属两个
    /// 时间尺度——今天心情差可以不听你的，但长期关系好的话，她拒绝的方式会更软。
    /// 50 是中性起点：她本来就是「女儿」，关系不从零开始
    #[serde(default = "default_affection")]
    pub affection: f32,
    /// 健康 0–100。**慢变量**：饿着 / 渴着 / 累着（低于 `UNWELL`）时往下掉，
    /// 什么都不缺时慢慢养回来；掉到 `SICK_HEALTH` 以下就养不回来了，得喂药。
    /// 它和心情分开——心情是「今天过得好不好」，健康是「这段日子有没有被照顾」
    #[serde(default = "default_health")]
    pub health: f32,
    /// 还没吸收完的药效（健康点数）。药不是一口见底，按 `REMEDY_PER_MIN` 慢慢起作用
    #[serde(default)]
    pub remedy: f32,
    pub action: Option<ActionRef>,
    pub updated_at: i64,
}

fn default_affection() -> f32 {
    50.0
}

fn default_health() -> f32 {
    100.0
}

impl Default for PetState {
    fn default() -> Self {
        Self {
            activity: Activity::Idle,
            mood: Mood::Nomal,
            strength: 100.0,
            feeling: 60.0,
            hunger: 100.0,
            thirst: 100.0,
            money: 0.0,
            exp: 0.0,
            level: 0,
            affection: default_affection(),
            health: default_health(),
            remedy: 0.0,
            action: None,
            updated_at: 0,
        }
    }
}

/// Core 内部的完整状态 = 发给 Body 的那部分 + 记账。
/// 记账不进线上契约：Body 不需要知道「这一轮已经赚了多少」。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pet {
    pub state: PetState,
    /// 当前动作已经做了多少分钟
    pub elapsed: f32,
    /// 这一轮已赚到的钱/经验，用来算完成奖励
    pub earned: f32,
    /// 动作 id → 还要等多少分钟才能再做
    pub cooldowns: HashMap<String, f32>,
    /// 番茄钟把她按在某类事情上（专注 → work，休息 → rest）。
    /// 生理急需仍然能压过它——番茄钟不该把人饿死
    pub pinned_tag: Option<String>,
    /// 被拒绝之后又被反复要求同一件事，攒下来的压力。随时间散掉。
    /// 存在的理由：拒绝必须有分量，否则用户只要连点就能把概率刷穿
    #[serde(default)]
    pub pressure: f32,
    /// 抚摸的额度（漏桶）。摸一下扣一点，随时间回。
    /// 边际效用递减是真实的——狂点头不该等于爱
    #[serde(default = "default_touch_budget")]
    pub touch_budget: f32,
    /// 最近一次服从判定的结果。lib.rs 读它发气泡，不进 `PetState` 契约
    #[serde(default)]
    pub last_verdict: Option<Verdict>,
    /// 用户的长期偏好：「多工作一点」「少玩会儿」。带半衰期，会自己淡掉
    #[serde(default)]
    pub biases: Biases,
    /// 用户刚开口要她做的那件事，她答应了——在这段时间里作息不会把她拽走。
    /// 和 `pinned_tag`（番茄钟）的区别是**有期限**：说一句「去玩会儿」不是让她玩一辈子
    #[serde(default)]
    pub directive: Option<Directive>,
}

/// 一次性请求答应之后的「保护期」：`target` 是动作 id 或 tag，`left` 是还剩多少分钟。
/// 生理急需照样能压过它（饿垮了不会因为你说了「去玩」就饿着）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Directive {
    pub target: String,
    pub left: f32,
}

/// 没说做多久时按这个算：有时长的动作做完它这一轮（最长 `MAX_DIRECTIVE_MIN`），
/// 不限时的（发呆）给二十分钟。「让她休息」不能变成「让她永远休息」
pub const DEFAULT_DIRECTIVE_MIN: f32 = 20.0;
pub const MAX_DIRECTIVE_MIN: f32 = 90.0;
/// 用户亲口说的时长也有上限：「玩一整天」按四个小时算
pub const MAX_REQUESTED_MIN: f32 = 240.0;

fn default_touch_budget() -> f32 {
    TOUCH_BUDGET_MAX
}

/// 手写而不是 `#[derive(Default)]`：`#[serde(default = "…")]` 只在**反序列化**时生效，
/// `Pet::default()` 走不到它。derive 会让 `touch_budget` 落成 0——
/// 新宠物一上来就「摸腻了」。同一个坑 `Requires::max_strength` 已经踩过一次
impl Default for Pet {
    fn default() -> Self {
        Self {
            state: PetState::default(),
            elapsed: 0.0,
            earned: 0.0,
            cooldowns: HashMap::new(),
            pinned_tag: None,
            pressure: 0.0,
            touch_budget: TOUCH_BUDGET_MAX,
            last_verdict: None,
            biases: Biases::default(),
            directive: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    Head,
    Body,
    Raise,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// 时间流逝。`hour` 是当前钟点（0–24），决定「到点该做什么」
    Tick { minutes: f32, hour: f32 },
    Touched(Touch),
    /// 番茄钟把她按在某类事情上；None = 松开
    Pin(Option<String>),
    /// 用户送了样东西。她自己不会凭空收到礼物，所以这条只能从外面来
    Gifted { id: String, name: String },
    /// 用户喂了她一样药（货架上 `Drug` 类）。她自己不会去买药——病了得有人照顾，
    /// 这是「需要用户」的那一环
    Medicated { id: String, name: String },
    /// 用户要求她做某件事。`target` 是动作 id 或 tag，`roll` 是外部喂的 0–1 随机数，
    /// `minutes` 是用户说的「做多久」（没说就按动作自己的时长）。
    /// **不保证执行**——要过 `obey::judge`
    Request { target: String, roll: f32, minutes: Option<f32> },
    /// 用户的长期偏好：正 = 多做，负 = 少做。`half_life` 是半衰期（分钟），
    /// 给 0 用默认值。这不是命令，只改她自己决策时的倾向
    SetBias { tag: String, weight: f32, half_life: f32 },
    /// 撤掉某条偏好；`None` = 全撤
    ClearBias(Option<String>),
    /// 调试用：直接改数值，用来验证阈值行为
    Patch {
        strength: Option<f32>,
        feeling: Option<f32>,
        hunger: Option<f32>,
        thirst: Option<f32>,
        money: Option<f32>,
        affection: Option<f32>,
        health: Option<f32>,
    },
}

/* --- 仲裁用的阈值。这些是「策略」，不是「数据」，所以留在代码里 --- */

/// 低到这个程度就压过作息：半夜也会爬起来吃
pub(crate) const STARVING: f32 = 20.0;
pub(crate) const PARCHED: f32 = 20.0;
pub(crate) const EXHAUSTED: f32 = 15.0;
/// 心情崩到这个程度，再忙也得歇一会儿——人不会把自己磨到彻底麻木还接着干
pub(crate) const MISERABLE: f32 = 15.0;
/// 没到饭点但也该补一口了
pub(crate) const HUNGRY: f32 = 35.0;
pub(crate) const THIRSTY: f32 = 35.0;
/// 心情低于此就想找点乐子
pub(crate) const SAD: f32 = 40.0;
/// 钱少于此就该去挣了
pub(crate) const BROKE: f32 = 80.0;

/* --- 迟滞：触发线在低位，**解除线在高位**。
   两条线合一是抖振的充要条件——心情 15 去玩，涨到 16 就被「上班时间」抢回去，
   掉回 15 又被抢走，两分钟翻一次。分开之后一个来回要几十分钟，
   看起来才像「歇够了再回去干活」。 --- */

/// 只有这三类听用户的「多做点 / 少做点」。
/// 吃喝睡不在里面——那是生理，用户关不掉，关掉就等于让她饿死
pub(crate) const SUPPRESSIBLE: [&str; 3] = ["work", "study", "play"];
/// 因为前置条件掉了（体力跌破门槛）而放弃的动作，先冷却这么久。不冷却的话
/// 歇几十秒体力回到门槛就又去干，一分钟后又跌下来，看起来像抽搐
const REQUIREMENT_COOLDOWN: f32 = 15.0;
/// 自己挑活干时，体力得比门槛高出这么多才**开始**（做起来之后掉到门槛才停）。
/// 开始线和放弃线分开，和心情那处迟滞是同一个道理——否则歇到刚够门槛就去，
/// 一分钟后又不够，再歇、再去
const START_MARGIN: f32 = 8.0;

/// 缓过这口气才算歇完
const RECOVERED_FEELING: f32 = 35.0;
const RECOVERED_STRENGTH: f32 = 40.0;
const RECOVERED_HUNGER: f32 = 45.0;
const RECOVERED_THIRST: f32 = 45.0;

const HAPPY_FEELING: f32 = 70.0;
const NOMAL_FEELING: f32 = 40.0;
const POOR_STRENGTH: f32 = 20.0;

/* --- 健康。比心情慢一个数量级：心情是今天，健康是这段日子 --- */

/// 饱腹 / 口渴 / 体力低于这条线就在伤身体
pub(crate) const UNWELL: f32 = 30.0;
/// 健康低于这条线算病了：面板黄条、心情大幅往下掉、自己养不回来，得喂药
pub(crate) const SICK_HEALTH: f32 = 50.0;
/// 低于这条线是 Ill：动画换成生病那套，正事一律干不动
pub(crate) const ILL_HEALTH: f32 = 25.0;
/// 每一项缺口每分钟掉多少健康。三项全缺 0.15/min，五个多小时从满掉到病线；
/// 只是上班累到 30 以下一两个小时，一天掉的还没养回来的多
const HEALTH_LOSS: f32 = 0.05;
/// 每一项缺口每分钟连带掉多少心情：饿着的时候不会开心
const FEELING_UNWELL: f32 = 0.05;
/// 病着的时候心情每分钟掉多少（一小时 12）。够「大幅」，也够让人看出来她不对劲
const FEELING_SICK: f32 = 0.2;
/// 什么都不缺、也没病时每分钟养回多少健康：一天回 30 左右，小磕小碰不用管
const HEALTH_REGEN: f32 = 0.02;
/// 药效每分钟吸收多少：一颗 65 的阿司匹林两个多小时见效完。「慢慢好转」
const REMEDY_PER_MIN: f32 = 0.5;
/// 病中被人喂药，关系往前推一大步——生病时谁在身边最记得住
const AFFECTION_NURSED: f32 = 0.5;

/* --- 好感度。慢变量，天级尺度 --- */

/// 摸头一次涨多少好感（要有额度）
const AFFECTION_HEAD: f32 = 0.15;
const AFFECTION_BODY: f32 = 0.08;
/// 她照做了，关系往前推一点
const AFFECTION_OBEY: f32 = 0.3;
/// 被逼急了掉多少
const AFFECTION_PUSHED: f32 = 0.5;
/// 抚摸额度上限与恢复速度（每分钟回多少）。摸 12 下就腻了，两小时回满
const TOUCH_BUDGET_MAX: f32 = 12.0;
const TOUCH_BUDGET_REGEN: f32 = 0.1;
/// 好感度回归的基准线与速率。不是惩罚，是「久不联系会淡」——
/// 指数回归，半衰期约七天（ln2 / (7×1440) ≈ 6.9e-5）
const AFFECTION_BASELINE: f32 = 40.0;
const AFFECTION_REGRESS: f32 = 6.9e-5;
/// 压力每分钟散掉多少：被拒一次攒 1 点，半小时散完。
/// 逼得越狠，缓过来越久——这是线性的，四次就是两小时
const PRESSURE_DECAY: f32 = 1.0 / 30.0;

const FEELING_HEAD: f32 = 1.0;
const FEELING_BODY: f32 = 0.5;
/// 被拎起来不太舒服，但也不惩罚（docs/01：情绪只正向放大）
const FEELING_RAISE: f32 = 0.0;
const STRENGTH_PER_TOUCH: f32 = 0.2;

/// 关掉期间最多按这么久补算。
///
/// 产品判断不是技术判断：离线衰减的目的是「感觉时间过去了」，不是「你冷落了我」。
/// 满饱腹跑到零只要几小时，上限必须明显短于它，否则每天早上打开都是一只饿到脱力的
/// 宠物——那是愧疚感机制，docs/01 写明情绪引擎只正向放大、不惩罚。
pub const MAX_CATCHUP_MIN: f32 = 2.0 * 60.0;

/// 经验到等级。等级解锁更赚钱的活，也直接给一点收入加成——
/// 「学习提高赚钱效率」这条因果链要看得见。
pub fn level_for(exp: f32) -> u32 {
    (exp.max(0.0) / 100.0).sqrt() as u32
}

/// 干活效率，移植自原版 MainLogic.cs:280-335：吃饱喝足才有高效率。
/// 返回值域 0.4–1.0
pub fn efficiency(s: &PetState) -> f32 {
    let one = |v: f32| {
        if v <= 25.0 {
            0.2 // 低状态低效率
        } else if v >= 60.0 {
            0.5
        } else {
            0.4
        }
    };
    one(s.hunger) + one(s.thirst)
}

/// 收入系数。原版：`addmoney = TimePass × MoneyBase × (2×efficiency − 0.5)`，
/// 我们额外乘一个等级加成，让「学习 → 赚更多」这条因果直接可见。
pub fn earn_multiplier(s: &PetState) -> f32 {
    let base = (2.0 * efficiency(s) - 0.5).max(0.0);
    base * (1.0 + s.level as f32 * 0.02)
}

/// 状态机主体。不设置 `updated_at`——那是时钟，由调用方填
pub fn reduce(cat: &Catalog, shelf: &FoodShelf, p: &Pet, e: &Event) -> Pet {
    let mut n = p.clone();
    // 判定结果只在产生它的那一拍有效，否则 lib.rs 会把同一句话发一整天
    n.last_verdict = None;
    match e {
        Event::Tick { minutes, hour } => tick(cat, shelf, &mut n, minutes.max(0.0), *hour),
        Event::Gifted { id, name } => {
            // 送礼是实打实的心意，按价钱折好感度（贵的更管用，但有上限）
            let price = shelf.get(id).map(|i| i.price).unwrap_or(0.0);
            n.state.affection += (price / 200.0).clamp(0.3, 3.0);
            // 收礼不走 decide：礼物是别人给的，不是她自己挑的
            if let Some(a) = cat.get("gift") {
                n.state.activity = a.activity;
                n.state.action = Some(ActionRef {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    graph: a.graph.clone(),
                    reason: "收到礼物了".into(),
                    food: Some(FoodRef {
                        id: id.clone(),
                        name: name.clone(),
                    }),
                });
                n.elapsed = 0.0;
                n.earned = 0.0;
            }
        }
        Event::Medicated { id, name } => {
            let item = shelf.get(id);
            // 药效进池子慢慢吸收；药自带的体力（钙片 +20）像买食物那样当场给
            n.state.remedy += item.map(|i| i.health).unwrap_or(0.0).max(0.0);
            n.state.strength += item.map(|i| i.strength).unwrap_or(0.0);
            if n.state.health < SICK_HEALTH {
                n.state.affection += AFFECTION_NURSED;
            }
            // 和收礼一样不走 decide：药是你喂的，不是她自己挑的。
            // 借「吃」的夹心动画把药片送进嘴里
            if let Some(a) = cat.get("medicine") {
                n.state.activity = a.activity;
                n.state.action = Some(ActionRef {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    graph: a.graph.clone(),
                    reason: "你喂的药".into(),
                    food: Some(FoodRef {
                        id: id.clone(),
                        name: name.clone(),
                    }),
                });
                n.elapsed = 0.0;
                n.earned = 0.0;
            }
            clamp(&mut n.state);
        }
        Event::Pin(tag) => {
            n.pinned_tag = tag.clone();
            // 立刻换过去，不等下一拍。你刚让她做的事还在保护期里就先不动
            if let (Some(t), None) = (tag, n.directive.as_ref()) {
                if let Some(a) = best(cat, t, |a| meets(a, &n.state)) {
                    let pick = (a, "番茄钟");
                    adopt(shelf, &mut n, pick);
                }
            }
        }
        Event::Touched(t) => {
            n.state.feeling += match t {
                Touch::Head => FEELING_HEAD,
                Touch::Body => FEELING_BODY,
                Touch::Raise => FEELING_RAISE,
            };
            n.state.strength -= STRENGTH_PER_TOUCH;
            // 好感度只在额度内涨：连点一百下和好好摸十下，效果不该一样
            if n.touch_budget >= 1.0 {
                n.touch_budget -= 1.0;
                n.state.affection += match t {
                    Touch::Head => AFFECTION_HEAD,
                    Touch::Body => AFFECTION_BODY,
                    Touch::Raise => 0.0,
                };
            }
            clamp(&mut n.state);
        }
        Event::Request { target, roll, minutes } => request(cat, shelf, &mut n, target, *roll, *minutes),
        Event::SetBias { tag, weight, half_life } => n.biases.set(tag, *weight, *half_life),
        Event::ClearBias(tag) => match tag {
            Some(t) => {
                n.biases.clear(t);
            }
            None => n.biases.clear_all(),
        },
        Event::Patch {
            strength,
            feeling,
            hunger,
            thirst,
            money,
            affection,
            health,
        } => {
            if let Some(v) = strength {
                n.state.strength = *v;
            }
            if let Some(v) = health {
                n.state.health = *v;
            }
            if let Some(v) = feeling {
                n.state.feeling = *v;
            }
            if let Some(v) = hunger {
                n.state.hunger = *v;
            }
            if let Some(v) = thirst {
                n.state.thirst = *v;
            }
            if let Some(v) = money {
                n.state.money = *v;
            }
            if let Some(v) = affection {
                n.state.affection = *v;
            }
            clamp(&mut n.state);
        }
    }
    n.state.mood = derive_mood(&n.state);
    n
}

fn tick(cat: &Catalog, shelf: &FoodShelf, n: &mut Pet, minutes: f32, hour: f32) {
    // 1. 冷却倒数
    n.cooldowns.retain(|_, left| {
        *left -= minutes;
        *left > 0.0
    });

    // 1.5 慢变量：好感度向基准线回归，压力散掉，抚摸额度回一点。
    //     放在数值推进之前，这样同一拍里的请求判定用的是已经衰减过的压力
    n.state.affection += (AFFECTION_BASELINE - n.state.affection) * AFFECTION_REGRESS * minutes;
    n.pressure = (n.pressure - PRESSURE_DECAY * minutes).max(0.0);
    n.biases.decay(minutes);
    n.touch_budget = (n.touch_budget + TOUCH_BUDGET_REGEN * minutes).min(TOUCH_BUDGET_MAX);
    // 「去玩会儿」的保护期到点就撤，之后她自己决定接下来干嘛
    if let Some(d) = n.directive.as_mut() {
        d.left -= minutes;
        if d.left <= 0.0 {
            n.directive = None;
        }
    }

    // 2. 当前动作把数值往前推
    let current = n
        .state
        .action
        .as_ref()
        .and_then(|a| cat.get(&a.id))
        .unwrap_or_else(|| cat.fallback());
    let d = current.per_min;
    // 吃喝时以「她买的那样东西」的数值为准；还没拿到食物目录时退回动作表自带的量
    let (dh, dt, df) = match n.state.action.as_ref().and_then(|a| a.food.as_ref()) {
        Some(f) => match shelf.get(&f.id) {
            Some(item) => {
                let dur = current.duration.max(0.01);
                // 礼物的 feeling 是原版的大尺度（30~1790，原版上限随等级涨），
                // 缩到我们的 0–100 上：便宜的聊胜于无，贵的直接把心情拉满
                let feel = if item.kind == "Gift" {
                    item.feeling / 10.0
                } else {
                    item.feeling
                };
                (item.strength_food / dur, item.strength_drink / dur, feel / dur)
            }
            None => (d.hunger, d.thirst, d.feeling),
        },
        None => (d.hunger, d.thirst, d.feeling),
    };
    n.state.strength += d.strength * minutes;
    n.state.hunger += dh * minutes;
    n.state.thirst += dt * minutes;
    n.state.feeling += df * minutes;

    // 2.5 健康：饿着 / 渴着 / 累着都在伤身体，也连带心情；病了心情掉得更快。
    //     什么都不缺、又没病，才慢慢养回来——病了靠自己养不好，得喂药（remedy）
    let lacking = [n.state.hunger, n.state.thirst, n.state.strength]
        .iter()
        .filter(|v| **v < UNWELL)
        .count() as f32;
    if lacking > 0.0 {
        n.state.health -= HEALTH_LOSS * lacking * minutes;
        n.state.feeling -= FEELING_UNWELL * lacking * minutes;
    } else if n.state.health >= SICK_HEALTH {
        n.state.health += HEALTH_REGEN * minutes;
    }
    if n.state.health < SICK_HEALTH {
        n.state.feeling -= FEELING_SICK * minutes;
    }
    let dose = n.state.remedy.min(REMEDY_PER_MIN * minutes);
    n.state.health += dose;
    n.state.remedy -= dose;

    let mult = earn_multiplier(&n.state);
    let money = current.earns.money * mult * minutes;
    let exp = current.earns.exp * mult * minutes;
    n.state.money += money;
    n.state.exp += exp;
    n.earned += money + exp;

    let finished_id = current.id.clone();
    let finish_bonus = current.finish;
    let duration = current.duration;
    let cooldown = current.cooldown;
    let earns_money = current.earns.money > 0.0;

    clamp(&mut n.state);
    n.state.level = level_for(n.state.exp);
    n.elapsed += minutes;

    // 3. 做完了就结算，然后挑下一件事
    let done = duration > 0.0 && n.elapsed >= duration;
    if done {
        if finish_bonus > 0.0 && n.earned > 0.0 {
            let bonus = n.earned * finish_bonus;
            if earns_money {
                n.state.money += bonus;
            } else {
                n.state.exp += bonus;
                n.state.level = level_for(n.state.exp);
            }
        }
        if cooldown > 0.0 {
            n.cooldowns.insert(finished_id, cooldown);
        }
        n.state.action = None;
        n.elapsed = 0.0;
        n.earned = 0.0;
        clamp(&mut n.state);
    }

    // 4. 过场动画不许打断；其余情况没事做或该换事做时重新决策
    let busy = n.state.activity.is_transient() && !done;
    if !busy && (n.state.action.is_none() || should_switch(cat, n, hour)) {
        // 是前置条件掉了才走的话，让这条活冷却一会儿，别在门槛上来回抽
        if let Some(cur) = n.state.action.as_ref().and_then(|a| cat.get(&a.id)) {
            if !meets(cur, &n.state) && cur.cooldown > 0.0 {
                n.cooldowns.insert(cur.id.clone(), REQUIREMENT_COOLDOWN.max(cur.cooldown));
            }
        }
        // 你刚让她做的事排在番茄钟前面：番茄钟也是你开的，而这句话是你刚说的
        let hold = n
            .directive
            .as_ref()
            .map(|d| (d.target.as_str(), "你让我做的"))
            .or(n.pinned_tag.as_deref().map(|t| (t, "番茄钟")));
        let chosen = decide_with_pin(cat, &n.state, &n.cooldowns, hour, hold, &n.biases);
        adopt(shelf, n, chosen);
    }
}

/// `target` 既可以是动作 id（`work_copy`）也可以是 tag（`play`）
pub(crate) fn matches_target(a: &ActionDef, target: &str) -> bool {
    a.id == target || a.has_tag(target)
}

/// 正在做的事还合不合适。只在「明显不该再做了」时才打断，
/// 否则她会每一拍都换一件事做，看起来像多动症。
fn should_switch(cat: &Catalog, n: &Pet, hour: f32) -> bool {
    let Some(cur) = n.state.action.as_ref().and_then(|a| cat.get(&a.id)) else {
        return true;
    };
    // 生理急需和情绪崩溃压过一切
    if n.state.hunger < STARVING || n.state.thirst < PARCHED || n.state.strength < EXHAUSTED {
        return !cur.has_tag("need") && !cur.has_tag("sleep");
    }
    if n.state.feeling < MISERABLE {
        return !cur.has_tag("cheer") || bedridden(cur, &n.state);
    }
    // 病重了正事一律放下，去躺着
    if bedridden(cur, &n.state) {
        return true;
    }
    // 你刚让她做的事，保护期内只要还在做就别换
    if let Some(d) = n.directive.as_ref() {
        return !matches_target(cur, &d.target);
    }
    // 番茄钟按着的时候，只要还在该做的那一类里就别换
    if let Some(tag) = n.pinned_tag.as_deref() {
        return !cur.has_tag(tag);
    }
    // 正在缓这口气就别打断：解除线比触发线高一截（见上面的迟滞注释）。
    // 放在生理急需之后——真出大事了照样能把她拽走
    for (tag, value, recovered) in [
        ("cheer", n.state.feeling, RECOVERED_FEELING),
        ("sleep", n.state.strength, RECOVERED_STRENGTH),
        ("eat", n.state.hunger, RECOVERED_HUNGER),
        ("drink", n.state.thirst, RECOVERED_THIRST),
    ] {
        if cur.has_tag(tag) && value < recovered {
            return false;
        }
    }

    // 你刚说了少做这个，那就别接着做了。生理类不受此限——
    // 「少吃点」不该让她饿死，这条在 decide 里由 SUPPRESSIBLE 白名单守着，
    // 这里只认那三类
    if SUPPRESSIBLE
        .iter()
        .any(|t| cur.has_tag(t) && n.biases.weight(t) <= SUPPRESS)
    {
        return true;
    }
    // 闲着发呆不算「在做事」：到点了、缺什么了、你说过想让她多做点什么，随时起来去做。
    // 每一拍重新决策不会抖——挑中的还是发呆时 adopt 是空操作
    if cur.duration <= 0.0 && !cur.is_scheduled() {
        return true;
    }
    // 排了时段的事，过了点就收手（下班了就别干了）
    if cur.is_scheduled() && !cur.fits_hour(hour) {
        return true;
    }
    // 前置条件掉下去了（累到干不动这活）
    !meets(cur, &n.state)
}

/// 挑下一件事。优先级从上到下，第一个满足的胜出。
///
/// 这个顺序就是「像人一样生活」的定义：
/// 生理急需 → 到点该做的 → 没到点但需要的 → 兜底发呆。
pub fn decide<'a>(
    cat: &'a Catalog,
    s: &PetState,
    cd: &HashMap<String, f32>,
    hour: f32,
) -> (&'a ActionDef, &'static str) {
    decide_with_pin(cat, s, cd, hour, None, &Biases::default())
}

/// 番茄钟跑着 / 你刚让她做某件事的时候，除了生理急需，其余一律听它的。
/// `hold` = (动作 id 或 tag, 给 `ActionRef.reason` 的理由)
pub fn decide_with_pin<'a>(
    cat: &'a Catalog,
    s: &PetState,
    cd: &HashMap<String, f32>,
    hour: f32,
    hold: Option<(&str, &'static str)>,
    biases: &Biases,
) -> (&'a ActionDef, &'static str) {
    // 正事和玩要「缓过来」才开始：门槛之上再留一段余量。吃喝睡不加——那是需要，不是选择
    let ok = |a: &ActionDef| {
        meets(a, s)
            && !bedridden(a, s)
            && !cd.contains_key(&a.id)
            && (!SUPPRESSIBLE.iter().any(|t| a.has_tag(t)) || s.strength >= a.requires.min_strength + START_MARGIN)
    };
    // 急需时连冷却都不管——真饿了不会因为「刚吃过」就饿着
    let urgent = |a: &ActionDef| meets(a, s);

    // 1. 生理急需：压过作息，半夜也会爬起来
    if s.hunger < STARVING {
        if let Some(a) = best(cat, "eat", urgent) {
            return (a, "饿得受不了");
        }
    }
    if s.thirst < PARCHED {
        if let Some(a) = best(cat, "drink", urgent) {
            return (a, "渴得受不了");
        }
    }
    if s.strength < EXHAUSTED {
        if let Some(a) = best(cat, "sleep", urgent) {
            return (a, "累垮了");
        }
    }
    if s.feeling < MISERABLE {
        if let Some(a) = best(cat, "cheer", |a| meets(a, s) && !bedridden(a, s)) {
            return (a, "实在撑不住了");
        }
    }

    // 番茄钟 / 你让她做的事：生理这关过了就听它的，作息和心情都往后排
    if let Some((target, why)) = hold {
        if let Some(a) = best_matching(cat, target, |a| meets(a, s) && !bedridden(a, s)) {
            return (a, why);
        }
    }

    // 2a. 到点该睡该吃。**用户偏好排不过这一层**——
    //     「多工作一点」不等于「别睡觉也别吃饭」。这条是安全边界，别挪到下面去
    for (tag, why) in [("sleep", "到点睡觉"), ("eat", "到饭点了")] {
        if let Some(a) = best(cat, tag, |a| a.is_scheduled() && a.fits_hour(hour) && ok(a)) {
            return (a, why);
        }
    }

    // 2b. 正事和玩：这三类才听用户的。按偏好重排（稳定排序，没偏好时就是原来的顺序），
    //     正偏置够大的还能越出自己的时段——「多工作」会让她晚上也想干活
    let mut lanes = [
        ("work", "上班时间", "你说要多工作"),
        ("study", "该学习了", "你说要多学习"),
        ("play", "该放松一下", "你说要多玩会儿"),
    ];
    lanes.sort_by(|x, y| {
        biases
            .weight(y.0)
            .partial_cmp(&biases.weight(x.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (tag, why, biased_why) in lanes {
        let w = biases.weight(tag);
        if w <= SUPPRESS {
            continue; // 你说了少做这个
        }
        let spill = w >= SPILL;
        if let Some(a) = best(cat, tag, |a| {
            a.is_scheduled() && (spill || a.fits_hour(hour)) && ok(a)
        }) {
            let reason = if a.fits_hour(hour) { why } else { biased_why };
            return (a, reason);
        }
    }

    // 3. 没到点，但身上的数值提出了要求
    if s.hunger < HUNGRY {
        if let Some(a) = best(cat, "eat", ok) {
            return (a, "有点饿了");
        }
    }
    if s.thirst < THIRSTY {
        if let Some(a) = best(cat, "drink", ok) {
            return (a, "有点渴了");
        }
    }
    if s.feeling < SAD {
        if let Some(a) = best(cat, "cheer", ok) {
            return (a, "心情不好，找点乐子");
        }
    }
    if s.money < BROKE {
        if let Some(a) = best(cat, "work", ok) {
            return (a, "钱不够了");
        }
    }

    // 4. 什么都不缺，但你说过想让她多做点什么。
    //    放在需求之后——她自己的需要比你的偏好优先，这是「角色」不是「工具」
    if let Some((tag, _)) = biases.strongest() {
        if let Some(a) = best(cat, tag, ok) {
            return (a, "你说要多做这个");
        }
    }

    // 5. 什么都不缺，发呆
    (cat.fallback(), "没什么事")
}

/// 同一标签里挑「最好」的一个：优先级看收益，收益一样看等级门槛高的
/// （门槛高的通常更强，能做就做）
pub(crate) fn best<'a>(
    cat: &'a Catalog,
    tag: &str,
    pred: impl Fn(&ActionDef) -> bool,
) -> Option<&'a ActionDef> {
    cat.all()
        .iter()
        .filter(|a| a.has_tag(tag) && pred(a))
        .max_by(|x, y| {
            let score = |a: &ActionDef| a.earns.money + a.earns.exp / 10.0 + a.requires.level as f32;
            score(x).partial_cmp(&score(y)).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// 和 `best` 一样，但 `target` 可以是动作 id：指名道姓的就只看那一条
pub(crate) fn best_matching<'a>(
    cat: &'a Catalog,
    target: &str,
    pred: impl Fn(&ActionDef) -> bool,
) -> Option<&'a ActionDef> {
    match cat.get(target) {
        Some(a) => pred(a).then_some(a),
        None => best(cat, target, pred),
    }
}

pub(crate) fn meets(a: &ActionDef, s: &PetState) -> bool {
    let r = a.requires;
    s.level >= r.level
        && s.strength >= r.min_strength
        && s.hunger >= r.min_hunger
        && s.thirst >= r.min_thirst
        && s.strength <= r.max_strength
}

/// 病重（Ill）的时候正事和玩一律干不动。吃喝睡不在此列——病人也要吃饭
pub(crate) fn bedridden(a: &ActionDef, s: &PetState) -> bool {
    s.health < ILL_HEALTH && SUPPRESSIBLE.iter().any(|t| a.has_tag(t))
}

/// 用户开口要她做某件事。`target` 可以是动作 id（`work_copy`），
/// 也可以是 tag（`work` / `rest` / `study`）——tag 走和 `decide` 同一套挑选逻辑，
/// 所以「去工作」和她自己决定去工作，挑中的是同一件活。
///
/// **这里是唯一一处「用户意志」进入状态机的入口**，而它进来之后立刻被降格成一次
/// 概率判定：用户不能直接写 `state.activity`。
fn request(cat: &Catalog, shelf: &FoodShelf, n: &mut Pet, target: &str, roll: f32, minutes: Option<f32>) {
    let picked = cat
        .get(target)
        .or_else(|| best(cat, target, |a| meets(a, &n.state)))
        // 一件都不满足条件时也要挑一个出来，好让 judge 回一句「我做不来」，
        // 而不是含糊的「这个我不会」
        .or_else(|| best(cat, target, |_| true));
    let Some(a) = picked else {
        n.last_verdict = Some(Verdict::refused(target, Refusal::Unknown, 0.0));
        return;
    };

    let mut v = judge(a, &n.state, n.pressure, roll);
    if v.obey {
        n.pressure = 0.0;
        n.state.affection += AFFECTION_OBEY;
        // 你开口了就别让冷却卡着：冷却是「她自己不会连着做」，不是「不能做」
        n.cooldowns.remove(&a.id);
        // 保护期：你说了多久就多久，没说就做完这一轮。**一定有期限**——
        // 「去休息」不能变成「永远休息」
        let left = match minutes.filter(|m| *m > 0.0) {
            Some(m) => m.min(MAX_REQUESTED_MIN),
            None if a.duration > 0.0 => a.duration.min(MAX_DIRECTIVE_MIN),
            None => DEFAULT_DIRECTIVE_MIN,
        };
        n.directive = Some(Directive { target: target.to_string(), left });
        v.minutes = Some(left);
        adopt(shelf, n, (a, "你让我做的"));
    } else {
        // 第一次拒绝不收费。拒了还接着逼，才开始伤心情和好感
        if n.pressure >= PRESSURE_HURT {
            n.state.feeling -= 2.0;
            n.state.affection -= AFFECTION_PUSHED;
        }
        n.pressure += 1.0;
    }
    clamp(&mut n.state);
    n.last_verdict = Some(v);
}

fn adopt(shelf: &FoodShelf, n: &mut Pet, (a, reason): (&ActionDef, &'static str)) {
    let same = n.state.action.as_ref().is_some_and(|c| c.id == a.id);
    if same {
        return;
    }
    // 吃喝就得先买。挑什么取决于缺什么——心情不好的时候买的东西不一样
    let food = buy(shelf, a, &mut n.state);
    n.state.activity = a.activity;
    n.state.action = Some(ActionRef {
        id: a.id.clone(),
        name: a.name.clone(),
        graph: a.graph.clone(),
        reason: reason.to_string(),
        food,
    });
    n.elapsed = 0.0;
    n.earned = 0.0;
}

/// 挑一样买下来，扣钱。买不起就返回 None——调用方退回动作表自带的量，
/// 免得钱包空了就把自己饿死（家里总还有点存粮）。
fn buy(shelf: &FoodShelf, a: &ActionDef, s: &mut PetState) -> Option<FoodRef> {
    let graph = if a.has_tag("drink") {
        "drink"
    } else if a.has_tag("eat") {
        "eat"
    } else {
        return None;
    };
    let need = if graph == "drink" {
        Need::Thirst
    } else if s.feeling < SAD {
        Need::Mood // 心情差就买点好的哄自己
    } else {
        Need::Hunger
    };
    let item = shelf.pick(graph, need, s.money)?;
    s.money = (s.money - item.price).max(0.0);
    s.strength = (s.strength + item.strength).clamp(0.0, 100.0);
    Some(FoodRef {
        id: item.id.clone(),
        name: item.name.clone(),
    })
}

/// 把「上次记录到现在」这段离线时间一次性补上。
///
/// 这是 `reduce` 保持纯函数换来的直接好处：补两小时和跑两小时走的是同一段代码。
pub fn catch_up(cat: &Catalog, shelf: &FoodShelf, p: &Pet, now_ms: i64, hour: f32) -> Pet {
    let minutes = (now_ms - p.state.updated_at).max(0) as f32 / 60_000.0;
    if minutes < 1.0 {
        return p.clone();
    }
    reduce(
        cat,
        shelf,
        p,
        &Event::Tick {
            minutes: minutes.min(MAX_CATCHUP_MIN),
            hour,
        },
    )
}

/// 四套动画对应的四种状态（docs/03 §3）：健康 < 25 → Ill；体力 < 20 或心情 < 40 →
/// PoorCondition；心情 ≥ 70 且没病 → Happy；其余 Nomal。病着（健康 < 50）再开心也
/// 只到 Nomal——脸色摆在那里
fn derive_mood(s: &PetState) -> Mood {
    if s.health < ILL_HEALTH {
        Mood::Ill
    } else if s.strength < POOR_STRENGTH || s.feeling < NOMAL_FEELING {
        Mood::PoorCondition
    } else if s.feeling >= HAPPY_FEELING && s.health >= SICK_HEALTH {
        Mood::Happy
    } else {
        Mood::Nomal
    }
}

fn clamp(s: &mut PetState) {
    for v in [
        &mut s.strength,
        &mut s.feeling,
        &mut s.hunger,
        &mut s.thirst,
        &mut s.affection,
        &mut s.health,
    ] {
        *v = v.clamp(0.0, 100.0);
    }
    s.remedy = s.remedy.max(0.0);
    s.money = s.money.max(0.0);
    s.exp = s.exp.max(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        Catalog::load()
    }

    /// 单元测试默认用空货架：买不起就退回动作表自带的量，
    /// 这样绝大多数测试不用关心食物，只有专门测购买的才装货
    fn shelf() -> FoodShelf {
        FoodShelf::default()
    }

    fn stocked() -> FoodShelf {
        use super::super::food::FoodItem;
        let mk = |id: &str, graph: &str, kind: &str, food: f32, drink: f32, feel: f32, price: f32| FoodItem {
            id: id.into(),
            name: id.into(),
            graph: graph.into(),
            kind: kind.into(),
            strength: 0.0,
            strength_food: food,
            strength_drink: drink,
            feeling: feel,
            health: 0.0,
            price,
        };
        let mut s = FoodShelf::default();
        s.set(vec![
            mk("bun", "eat", "Meal", 40.0, 0.0, 1.0, 10.0),
            mk("cake", "eat", "Snack", 8.0, 0.0, 40.0, 25.0),
            mk("tea", "drink", "Drink", 0.0, 45.0, 2.0, 6.0),
        ]);
        s
    }

    /// 跑 n 分钟，每分钟一拍，钟点跟着走
    fn run(cat: &Catalog, mut p: Pet, minutes: u32, start_hour: f32) -> Pet {
        for i in 0..minutes {
            let hour = (start_hour + i as f32 / 60.0) % 24.0;
            p = reduce(cat, &shelf(), &p, &Event::Tick { minutes: 1.0, hour });
        }
        p
    }

    fn run_with(cat: &Catalog, sh: &FoodShelf, mut p: Pet, minutes: u32, start_hour: f32) -> Pet {
        for i in 0..minutes {
            let hour = (start_hour + i as f32 / 60.0) % 24.0;
            p = reduce(cat, sh, &p, &Event::Tick { minutes: 1.0, hour });
        }
        p
    }

    fn acting(p: &Pet) -> String {
        p.state.action.as_ref().map(|a| a.id.clone()).unwrap_or_default()
    }

    #[test]
    fn 半夜会去睡觉() {
        let c = cat();
        let p = run(&c, Pet::default(), 5, 23.5);
        assert_eq!(p.state.activity, Activity::Sleeping, "凌晨该睡了");
    }

    /* ---------- 门槛上不抽搐（换花样是 Body 的事：动画池，见 05 §8） ---------- */

    #[test]
    fn 体力在门槛上不会来回抽搐() {
        let c = cat();
        let mut p = Pet::default();
        p.state.strength = 20.5; // 文案要 20，刚够
        p.state.money = 0.0;
        let mut flips = 0;
        let mut last = String::new();
        for i in 0..40 {
            p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.0 + i as f32 / 60.0 });
            let id = p.state.action.as_ref().unwrap().id.clone();
            if id != last {
                flips += 1;
                last = id;
            }
        }
        assert!(flips <= 4, "四十分钟换了 {flips} 次");
    }

    #[test]
    fn 早上八点会起床_哪怕时钟停过() {
        // 23 点躺下，电脑随后待机到早上——期间一拍都没走。醒来第一拍在 8 点之后，
        // 不管这一觉「按拍数」睡够没有，作息都该把她叫起来
        let c = cat();
        let mut p = run(&c, Pet::default(), 3, 23.0);
        assert_eq!(p.state.activity, Activity::Sleeping, "前提：23 点该睡了");
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0 / 60.0, hour: 8.05 });
        assert_ne!(p.state.activity, Activity::Sleeping, "八点过了还在睡");
        // 七点半还在睡觉时段里
        let mut q = run(&c, Pet::default(), 3, 23.0);
        q = reduce(&c, &shelf(), &q, &Event::Tick { minutes: 1.0 / 60.0, hour: 7.5 });
        assert_eq!(q.state.activity, Activity::Sleeping, "七点半该还在睡");
    }

    #[test]
    fn 待机醒来会把这段时间补上() {
        // 和重启时一样走 catch_up：待机八小时，饱腹要掉、睡觉要睡够、到点要起床
        let c = cat();
        let mut p = run(&c, Pet::default(), 3, 23.0);
        p.state.updated_at = 0;
        let woke = catch_up(&c, &shelf(), &p, 8 * 60 * 60 * 1000, 9.0);
        assert_ne!(woke.state.activity, Activity::Sleeping, "九点了还在睡");
        assert!(woke.state.hunger < p.state.hunger, "待机期间也会饿");
    }

    #[test]
    fn 上班时间会去工作() {
        let c = cat();
        let p = run(&c, Pet::default(), 5, 10.0);
        assert_eq!(p.state.activity, Activity::Working, "上午十点该干活");
    }

    #[test]
    fn 到饭点会吃饭而不用等饿() {
        let c = cat();
        // 饱腹还满着，按阈值逻辑根本不会吃——但到点了就该吃
        let p = run(&c, Pet::default(), 1, 12.0);
        assert_eq!(p.state.activity, Activity::Eating, "中午十二点该吃饭");
        assert_eq!(acting(&p), "meal");
    }

    #[test]
    fn 饿极了半夜也会爬起来吃() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 5.0;
        let p = run(&c, p, 1, 3.0); // 凌晨三点
        assert_eq!(p.state.activity, Activity::Eating, "生理急需要压过作息");
        assert_eq!(acting(&p), "snack");
    }

    #[test]
    fn 晚上会学习或者玩不会上班() {
        let c = cat();
        let p = run(&c, Pet::default(), 5, 20.0);
        assert_ne!(p.state.activity, Activity::Working, "晚上八点不该还在上班");
    }

    #[test]
    fn 吃完饱腹真的回来了() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 10.0;
        let before = p.state.hunger;
        let p = run(&c, p, 4, 12.0);
        assert!(p.state.hunger > before + 20.0, "实际 {}", p.state.hunger);
    }

    #[test]
    fn 工作会赚钱() {
        let c = cat();
        let p = run(&c, Pet::default(), 30, 10.0);
        assert!(p.state.money > 0.0, "上午干了半小时该有收入");
    }

    #[test]
    fn 学习会涨经验和等级() {
        let c = cat();
        let mut p = Pet::default();
        p.state.money = 9999.0; // 不缺钱，免得跑去工作
        let p = run(&c, p, 40, 20.0);
        assert!(p.state.exp > 0.0, "晚上学习该涨经验");
        assert!(p.state.level > 0, "经验够了该升级");
    }

    #[test]
    fn 吃饱喝足才有高效率() {
        let full = PetState {
            hunger: 90.0,
            thirst: 90.0,
            ..Default::default()
        };
        let empty = PetState {
            hunger: 10.0,
            thirst: 10.0,
            ..Default::default()
        };
        assert!(efficiency(&full) > efficiency(&empty));
        assert!(earn_multiplier(&full) > earn_multiplier(&empty) * 2.0);
    }

    #[test]
    fn 等级越高赚得越多() {
        let low = PetState {
            level: 0,
            ..Default::default()
        };
        let high = PetState {
            level: 30,
            ..Default::default()
        };
        assert!(
            earn_multiplier(&high) > earn_multiplier(&low),
            "学习提高赚钱效率这条因果要成立"
        );
    }

    #[test]
    fn 等级不够的活干不了() {
        let c = cat();
        let s = PetState::default(); // level 0
        let cd = HashMap::new();
        let (a, _) = decide(&c, &s, &cd, 10.0);
        assert!(a.requires.level <= s.level, "挑中了做不了的活：{}", a.name);
    }

    #[test]
    fn 冷却中的动作不会被重复挑中() {
        let c = cat();
        let s = PetState::default();
        let mut cd = HashMap::new();
        let (first, _) = decide(&c, &s, &cd, 10.0);
        cd.insert(first.id.clone(), 30.0);
        let (second, _) = decide(&c, &s, &cd, 10.0);
        assert_ne!(first.id, second.id, "冷却没生效");
    }

    /// 一天二十四小时都排了事（这本身是对的——人本来就有作息），
    /// 所以要测「需求驱动」这一层，得先把当下排定的事按掉
    fn 只剩需求驱动(hour: f32) -> HashMap<String, f32> {
        let c = cat();
        c.all()
            .iter()
            .filter(|a| a.is_scheduled() && a.fits_hour(hour))
            .map(|a| (a.id.clone(), 60.0))
            .collect()
    }

    #[test]
    fn 心情差会去找乐子() {
        let c = cat();
        let s = PetState {
            feeling: 10.0,
            money: 9999.0,
            ..Default::default()
        };
        let (a, why) = decide(&c, &s, &只剩需求驱动(2.0), 2.0);
        assert!(a.has_tag("cheer"), "挑了 {}（{why}）", a.name);
    }

    #[test]
    fn 没钱会去工作() {
        let c = cat();
        let s = PetState {
            money: 0.0,
            feeling: 80.0,
            ..Default::default()
        };
        let (a, why) = decide(&c, &s, &只剩需求驱动(2.0), 2.0);
        assert!(a.has_tag("work"), "挑了 {}（{why}）", a.name);
    }

    #[test]
    fn 什么都不缺就发呆() {
        let c = cat();
        let s = PetState {
            money: 9999.0,
            feeling: 90.0,
            ..Default::default()
        };
        let (a, _) = decide(&c, &s, &只剩需求驱动(2.0), 2.0);
        assert_eq!(a.id, "rest");
    }

    #[test]
    fn 心情崩了会中断工作去歇一会儿() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 5.0;
        // 上午十点本该上班
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
        let a = p.state.action.as_ref().unwrap();
        assert!(
            c.get(&a.id).unwrap().has_tag("cheer"),
            "心情只剩 5 还在 {}（{}）",
            a.name,
            a.reason
        );
    }

    #[test]
    fn 上班时段心情不会被磨到见底() {
        let c = cat();
        let mut p = Pet::default();
        let mut worst = 100.0f32;
        for i in 0..(10 * 60) {
            let hour = (9.0 + i as f32 / 60.0) % 24.0;
            p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour });
            worst = worst.min(p.state.feeling);
        }
        assert!(worst > 0.0, "从早九点干到晚七点，心情被磨到了 {worst}");
    }

    #[test]
    fn 半夜心情再差也是先睡觉() {
        // 作息压过心情调节：凌晨两点不该爬起来打游戏
        let c = cat();
        let s = PetState {
            feeling: 25.0, // 低于 SAD 但没到崩溃，作息说了算
            money: 9999.0,
            ..Default::default()
        };
        let (a, why) = decide(&c, &s, &HashMap::new(), 2.0);
        assert!(a.has_tag("sleep"), "挑了 {}（{why}）", a.name);
    }

    #[test]
    fn 每个决策都带理由() {
        let c = cat();
        let p = run(&c, Pet::default(), 3, 10.0);
        let reason = &p.state.action.as_ref().unwrap().reason;
        assert!(!reason.is_empty(), "数值变动要有迹可循");
    }

    #[test]
    fn 吃饭过程中不会被打断去上班() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 5.0;
        // 上午十点，本该上班；但饿到不行，先吃
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
        assert_eq!(p.state.activity, Activity::Eating);
        // 下一拍还在吃，不能被上班抢走
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 0.5, hour: 10.0 });
        assert_eq!(p.state.activity, Activity::Eating, "过场动画不许打断");
    }

    #[test]
    fn 不会每一拍都换一件事做() {
        let c = cat();
        let mut p = run(&c, Pet::default(), 2, 10.0);
        let first = acting(&p);
        let mut switches = 0;
        for _ in 0..20 {
            p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
            if acting(&p) != first {
                switches += 1;
            }
        }
        assert!(switches < 5, "二十分钟换了 {switches} 次，像多动症");
    }

    #[test]
    fn 数值永远不越界() {
        let c = cat();
        let p = run(&c, Pet::default(), 3000, 0.0);
        let s = &p.state;
        for v in [s.strength, s.feeling, s.hunger, s.thirst] {
            assert!((0.0..=100.0).contains(&v), "越界 {v}");
        }
        assert!(s.money >= 0.0 && s.exp >= 0.0);
    }

    #[test]
    fn 跑一整天不会卡死在某个状态() {
        let c = cat();
        let mut p = Pet::default();
        let mut seen = std::collections::HashSet::new();
        for i in 0..(24 * 60) {
            let hour = (i as f32 / 60.0) % 24.0;
            p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour });
            seen.insert(p.state.activity);
        }
        assert!(seen.len() >= 4, "一整天只出现了 {:?}", seen);
        assert!(seen.contains(&Activity::Sleeping), "一天都没睡觉");
        assert!(seen.contains(&Activity::Eating), "一天都没吃饭");
    }

    /* ---- 她自己挣钱、自己买东西 ---- */

    #[test]
    fn 吃饭会花钱并且买的东西记在案() {
        let c = cat();
        let mut p = Pet::default();
        p.state.money = 100.0;
        p.state.hunger = 5.0; // 饿到不行，一定会去吃
        let p = reduce(&c, &stocked(), &p, &Event::Tick { minutes: 1.0, hour: 12.0 });
        let a = p.state.action.as_ref().unwrap();
        let f = a.food.as_ref().expect("吃饭得先买一样东西");
        assert_eq!(f.id, "bun", "饿的时候该买管饱的");
        assert!(p.state.money < 100.0, "买了东西却没花钱");
    }

    #[test]
    fn 心情差的时候买的东西不一样() {
        let c = cat();
        let mut p = Pet::default();
        p.state.money = 100.0;
        p.state.hunger = 5.0;
        p.state.feeling = 5.0; // 又饿又难受
        let p = reduce(&c, &stocked(), &p, &Event::Tick { minutes: 1.0, hour: 12.0 });
        let f = p.state.action.as_ref().unwrap().food.as_ref().unwrap();
        assert_eq!(f.id, "cake", "心情差就该买点好的哄自己，而不是啃馒头");
    }

    #[test]
    fn 买的东西真的回饱腹() {
        let c = cat();
        let mut p = Pet::default();
        p.state.money = 100.0;
        p.state.hunger = 5.0;
        // 吃饭 duration=2，馒头回 40 → 两分钟吃完该到 45 上下
        let p = run_with(&c, &stocked(), p, 2, 12.0);
        assert!(p.state.hunger > 35.0, "实际 {}", p.state.hunger);
    }

    #[test]
    fn 没钱也不会把自己饿死() {
        let c = cat();
        let mut p = Pet::default();
        p.state.money = 0.0; // 一分钱没有
        p.state.hunger = 5.0;
        let p = run_with(&c, &stocked(), p, 2, 12.0);
        let a = p.state.action.as_ref().unwrap();
        assert!(a.food.is_none(), "没钱不该凭空变出食物");
        assert!(p.state.hunger > 5.0, "买不起也得退回动作表自带的量，不能饿死");
    }

    #[test]
    fn 货架空着照样能过日子() {
        // Body 还没把目录推过来的那段时间
        let c = cat();
        let p = run(&c, Pet::default(), 24 * 60, 0.0);
        assert!(p.state.hunger > 0.0);
    }

    #[test]
    fn 挣的钱够她自己吃饭() {
        let c = cat();
        let p = run_with(&c, &stocked(), Pet::default(), 24 * 60, 0.0);
        assert!(p.state.money > 0.0, "忙活一整天最后一分钱不剩，经济尺度不对");
    }

    #[test]
    fn 收礼物会涨心情而且不花她的钱() {
        let c = cat();
        let mut sh = stocked();
        {
            use super::super::food::FoodItem;
            sh.set(vec![FoodItem {
                id: "phone".into(),
                name: "APhone X".into(),
                graph: "gift".into(),
                kind: "Gift".into(),
                strength: 0.0,
                strength_food: 0.0,
                strength_drink: 0.0,
                feeling: 290.0,
                health: 0.0,
                price: 974.0,
            }]);
        }
        let mut p = Pet::default();
        p.state.feeling = 20.0;
        p.state.money = 10.0;
        let money_before = p.state.money;

        p = reduce(&c, &sh, &p, &Event::Gifted { id: "phone".into(), name: "APhone X".into() });
        assert_eq!(p.state.activity, Activity::Gift);
        assert_eq!(p.state.action.as_ref().unwrap().food.as_ref().unwrap().id, "phone");

        p = run_with(&c, &sh, p, 3, 15.0);
        assert!(p.state.feeling > 20.0, "收了礼物心情没变：{}", p.state.feeling);
        assert_eq!(p.state.money, money_before, "礼物是用户送的，不该扣她的钱");
    }

    #[test]
    fn 收礼物过程中不会被打断() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 5.0; // 就算饿着
        p = reduce(&c, &shelf(), &p, &Event::Gifted { id: "x".into(), name: "礼物".into() });
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 0.5, hour: 12.0 });
        assert_eq!(p.state.activity, Activity::Gift, "拆礼物拆到一半不该跑去吃饭");
    }

    #[test]
    fn 她自己不会凭空收到礼物() {
        let c = cat();
        let sh = stocked();
        // 跑一整天，decide 永远不该挑中 gift
        let mut p = Pet::default();
        for i in 0..(24 * 60) {
            let hour = (i as f32 / 60.0) % 24.0;
            p = reduce(&c, &sh, &p, &Event::Tick { minutes: 1.0, hour });
            assert_ne!(
                p.state.activity,
                Activity::Gift,
                "礼物得是别人给的，不能自己长出来"
            );
        }
    }

    /* ---- 番茄钟联动 ---- */

    #[test]
    fn 番茄钟按住她就去工作() {
        let c = cat();
        let mut p = Pet::default();
        // 半夜，本来该睡觉
        p = reduce(&c, &shelf(), &p, &Event::Pin(Some("work".into())));
        assert_eq!(p.state.activity, Activity::Working, "番茄钟说专注就该专注");
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 2.0 });
        assert_eq!(p.state.activity, Activity::Working, "作息不该把她拽去睡");
    }

    #[test]
    fn 番茄钟松开就回正常作息() {
        let c = cat();
        let mut p = Pet::default();
        p = reduce(&c, &shelf(), &p, &Event::Pin(Some("work".into())));
        p = reduce(&c, &shelf(), &p, &Event::Pin(None));
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 2.0 });
        assert_eq!(p.state.activity, Activity::Sleeping, "松开后凌晨两点该睡觉");
    }

    #[test]
    fn 番茄钟不该把人饿死() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 3.0;
        p = reduce(&c, &shelf(), &p, &Event::Pin(Some("work".into())));
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
        assert_eq!(
            p.state.activity,
            Activity::Eating,
            "生理急需要能压过番茄钟"
        );
    }

    #[test]
    fn 番茄钟休息相位会让她歇着() {
        let c = cat();
        let mut p = Pet::default();
        p = reduce(&c, &shelf(), &p, &Event::Pin(Some("rest".into())));
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
        let a = p.state.action.as_ref().unwrap();
        assert!(c.get(&a.id).unwrap().has_tag("rest"), "在 {}", a.name);
    }

    #[test]
    fn 摸头涨心情() {
        let c = cat();
        let p = Pet::default();
        let after = reduce(&c, &shelf(), &p, &Event::Touched(Touch::Head));
        assert!(after.state.feeling > p.state.feeling);
        assert!(after.state.strength < p.state.strength, "摸也要消耗一点体力");
    }

    #[test]
    fn 提起不扣心情() {
        let c = cat();
        let p = Pet::default();
        let after = reduce(&c, &shelf(), &p, &Event::Touched(Touch::Raise));
        assert!(
            after.state.feeling >= p.state.feeling,
            "情绪引擎只正向放大（docs/01）"
        );
    }

    #[test]
    fn 心情决定表情() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 80.0;
        assert_eq!(
            reduce(&c, &shelf(), &p, &Event::Touched(Touch::Raise)).state.mood,
            Mood::Happy
        );
        p.state.feeling = 20.0;
        assert_eq!(
            reduce(&c, &shelf(), &p, &Event::Touched(Touch::Raise)).state.mood,
            Mood::PoorCondition
        );
    }

    #[test]
    fn 体力过低直接算状态差() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 100.0;
        p.state.strength = POOR_STRENGTH - 1.0;
        let s = reduce(&c, &shelf(), &p, &Event::Touched(Touch::Raise));
        assert_eq!(s.state.mood, Mood::PoorCondition, "体力见底时心情再好也是状态差");
    }

    #[test]
    fn 离线期间照样会饿() {
        let c = cat();
        let p = Pet::default();
        let after = catch_up(&c, &shelf(), &p, 2 * 60 * 60 * 1000, 10.0);
        assert!(after.state.hunger < p.state.hunger, "关掉期间也该掉饱腹");
    }

    #[test]
    fn 离线太久不会把宠物饿死() {
        let c = cat();
        let p = Pet::default();
        let after = catch_up(&c, &shelf(), &p, 3 * 24 * 60 * 60 * 1000, 10.0);
        assert!(after.state.hunger > 0.0, "离线三天回来不该饿到脱力");
        assert_ne!(
            after.state.mood,
            Mood::PoorCondition,
            "长时间没开不该一上来就是坏心情（docs/01）"
        );
    }

    #[test]
    fn 刚存过就重启不会重复扣() {
        let c = cat();
        let mut p = Pet::default();
        p.state.updated_at = 1_000;
        p.state.hunger = 50.0;
        let after = catch_up(&c, &shelf(), &p, 1_500, 10.0);
        assert_eq!(after.state.hunger, 50.0);
    }

    #[test]
    fn 秒级tick和分钟级tick等价() {
        let c = cat();
        let by_min = run(&c, Pet::default(), 10, 3.0);
        let mut by_sec = Pet::default();
        for i in 0..600 {
            let hour = (3.0 + i as f32 / 3600.0) % 24.0;
            by_sec = reduce(
                &c,
                &shelf(),
                &by_sec,
                &Event::Tick {
                    minutes: 1.0 / 60.0,
                    hour,
                },
            );
        }
        assert!((by_min.state.hunger - by_sec.state.hunger).abs() < 1.0);
    }

    #[test]
    fn 序列化的字段名和前端契约一致() {
        let json = serde_json::to_string(&PetState::default()).unwrap();
        for key in [
            "activity", "mood", "strength", "feeling", "hunger", "thirst", "money", "exp", "level",
            "action", "updatedAt",
        ] {
            assert!(json.contains(key), "缺字段 {key}：{json}");
        }
        assert!(json.contains("\"idle\""));
        assert!(json.contains("\"nomal\""));
    }

    /* ---------- 用户偏好 Bias（roadmap 2.10） ---------- */

    fn biased(cat: &Catalog, tag: &str, w: f32) -> Pet {
        let mut p = Pet::default();
        p = reduce(cat, &shelf(), &p, &Event::SetBias { tag: tag.into(), weight: w, half_life: 120.0 });
        p
    }

    /// 某个钟点她会选什么
    fn at(cat: &Catalog, p: &Pet, hour: f32) -> String {
        let n = reduce(cat, &shelf(), p, &Event::Tick { minutes: 1.0, hour });
        n.state.action.as_ref().map(|a| a.id.clone()).unwrap_or_default()
    }

    #[test]
    fn 多学习会把学习排到工作前面() {
        let c = cat();
        let plain = Pet::default();
        let keen = biased(&c, "study", 1.0);
        // 上午既是上班时段也是学习时段，默认 work 先
        assert!(c.get(&at(&c, &plain, 10.0)).unwrap().has_tag("work"));
        assert!(c.get(&at(&c, &keen, 10.0)).unwrap().has_tag("study"), "说了多学习还在上班");
    }

    #[test]
    fn 多工作会让她越出上班时段() {
        let c = cat();
        let keen = biased(&c, "work", 1.0);
        let id = at(&c, &keen, 21.0); // 晚上九点，不是上班时段
        let a = c.get(&id).unwrap();
        assert!(a.has_tag("work"), "晚上九点她在 {}", a.name);
        assert!(!a.fits_hour(21.0), "这条测的就是越界");
    }

    #[test]
    fn 少玩点就真的不玩了() {
        let c = cat();
        let mut p = biased(&c, "play", -1.0);
        p.state.feeling = 60.0; // 心情正常，不会走「心情崩了」那条急需通道
        let id = at(&c, &p, 20.0); // 晚上八点本来是玩的时段
        assert!(!c.get(&id).unwrap().has_tag("play"), "说了少玩还在 {id}");
    }

    /* --- 下面三条是安全边界：偏好排不过生理。别删 --- */

    #[test]
    fn 多工作也不会不睡觉() {
        let c = cat();
        let keen = biased(&c, "work", 2.0);
        let id = at(&c, &keen, 1.0); // 凌晨一点
        assert!(c.get(&id).unwrap().has_tag("sleep"), "凌晨一点她在 {id}");
    }

    #[test]
    fn 多工作也不会不吃饭() {
        let c = cat();
        let keen = biased(&c, "work", 2.0);
        let id = at(&c, &keen, 12.0); // 饭点
        assert!(c.get(&id).unwrap().has_tag("eat"), "饭点她在 {id}");
    }

    #[test]
    fn 饿垮了偏好一点用都没有() {
        let c = cat();
        let mut keen = biased(&c, "work", 2.0);
        keen.state.hunger = 5.0;
        let n = reduce(&c, &shelf(), &keen, &Event::Tick { minutes: 1.0, hour: 10.0 });
        assert_eq!(n.state.activity, Activity::Eating);
    }

    #[test]
    fn 偏好压不掉吃喝睡() {
        let c = cat();
        // 就算用户把 eat 的权重按到底，也不该影响「到饭点了」
        let mut p = Pet::default();
        p = reduce(&c, &shelf(), &p, &Event::SetBias { tag: "eat".into(), weight: -2.0, half_life: 120.0 });
        let id = at(&c, &p, 12.0);
        assert!(c.get(&id).unwrap().has_tag("eat"), "「少吃点」把她饿着了：{id}");
    }

    #[test]
    fn 闲着的时候会去做你说的那件事() {
        let c = cat();
        let mut p = Pet::default();
        // 凌晨四点本该睡觉，先让她醒着发呆：体力满、刚睡过、钱也够
        p.cooldowns.insert("sleep".into(), 600.0);
        p.state.money = BROKE * 2.0;
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 4.0 });
        let idle = p.state.action.as_ref().unwrap().id.clone();
        assert_eq!(idle, c.fallback().id, "前提不成立，她在 {idle}");
        p = reduce(&c, &shelf(), &p, &Event::SetBias { tag: "study".into(), weight: 1.0, half_life: 120.0 });
        let id = at(&c, &p, 4.0);
        assert!(c.get(&id).unwrap().has_tag("study"), "闲着也不去学：{id}");
    }

    #[test]
    fn 偏好会自己淡掉() {
        let c = cat();
        let keen = biased(&c, "work", 1.0);
        assert!((keen.biases.weight("work") - 1.0).abs() < 1e-6);
        // 四个半衰期之后基本没了
        let later = run(&c, keen, 480, 10.0);
        assert!(later.biases.weight("work") < 0.1, "还剩 {}", later.biases.weight("work"));
    }

    #[test]
    fn 撤掉偏好() {
        let c = cat();
        let mut p = biased(&c, "work", 1.0);
        p = reduce(&c, &shelf(), &p, &Event::ClearBias(Some("work".into())));
        assert_eq!(p.biases.weight("work"), 0.0);
        p = biased(&c, "study", 1.0);
        p = reduce(&c, &shelf(), &p, &Event::ClearBias(None));
        assert!(p.biases.is_empty());
    }

    /* ---------- 迟滞：触发线和解除线分开 ---------- */

    #[test]
    fn 心情崩了之后不会在阈值上来回抽搐() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 14.0; // 刚跌破 MISERABLE
        // 下午三点，作息层会抢着让她上班——迟滞就是防这一下
        let mut flips = 0;
        let mut last = String::new();
        for i in 0..90 {
            p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 15.0 + i as f32 / 60.0 });
            let now = p.state.action.as_ref().map(|a| a.id.clone()).unwrap_or_default();
            if now != last {
                flips += 1;
                last = now;
            }
        }
        // 没有迟滞时这里会翻四十多次（每两分钟一次）
        assert!(flips < 8, "一个半小时里换了 {flips} 次事情做");
    }

    #[test]
    fn 歇到缓过来了才回去干活() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 14.0;
        // 先让她进入「找点乐子」
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 15.0 });
        let playing = p.state.action.as_ref().unwrap().id.clone();
        assert!(c.get(&playing).unwrap().has_tag("cheer"), "应该先去缓一缓，而不是 {playing}");
        // 心情刚爬过触发线还不能被拽走
        p.state.feeling = MISERABLE + 5.0;
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 15.1 });
        assert_eq!(p.state.action.as_ref().unwrap().id, playing, "刚过 15 就被抢走了");
        // 缓过解除线才松手
        p.state.feeling = RECOVERED_FEELING + 5.0;
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 15.2 });
        assert_ne!(p.state.action.as_ref().unwrap().id, playing, "缓过来了还赖着不走");
    }

    #[test]
    fn 缓着的时候真出大事照样拽得走() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 20.0; // 在触发线和解除线之间，正缓着
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 15.0 });
        p.state.hunger = 5.0; // 饿垮了
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 15.1 });
        assert_eq!(p.state.activity, Activity::Eating, "迟滞不该压过生理急需");
    }

    /* ---------- 好感度与服从（roadmap 2.8） ---------- */

    fn ask(cat: &Catalog, p: &Pet, target: &str, roll: f32) -> Pet {
        reduce(cat, &shelf(), p, &Event::Request { target: target.into(), roll, minutes: None })
    }

    fn ask_for(cat: &Catalog, p: &Pet, target: &str, minutes: f32) -> Pet {
        reduce(cat, &shelf(), p, &Event::Request { target: target.into(), roll: 0.0, minutes: Some(minutes) })
    }

    /* ---------- 使唤的保护期（directive） ---------- */

    #[test]
    fn 上班时间让她去玩她会玩够这一轮() {
        let c = cat();
        let mut p = run(&c, Pet::default(), 2, 10.0); // 上午十点，本来在上班
        p.state.feeling = 90.0;
        p = ask(&c, &p, "play", 0.0);
        assert_eq!(p.state.activity, Activity::Playing);
        let d = p.directive.as_ref().expect("答应了就该有保护期");
        assert_eq!(d.target, "play");
        assert!(d.left > 0.0 && d.left <= MAX_DIRECTIVE_MIN);
        // 二十分钟后还在玩：作息（上班时间）没能把她拽走
        let later = run(&c, p.clone(), 20, 10.05);
        assert_eq!(later.state.activity, Activity::Playing, "说了去玩，一分钟就被上班抢回去了");
        // 保护期过了就回到自己的安排（十一点还是上班时间，玩游戏要到十二点才合适）
        let after = run(&c, p, 60, 10.05);
        assert!(after.directive.is_none(), "保护期不会无限长");
        assert_eq!(after.state.activity, Activity::Working, "一小时后还在 {:?}：保护期没到点", after.state.activity);
    }

    #[test]
    fn 发呆的时候到点了会起来做事() {
        let c = cat();
        let mut p = Pet::default();
        p.state.money = 9999.0; // 不缺钱：八点半没别的事，只能发呆
        p.cooldowns.insert("meal".into(), 100.0);
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 8.5 });
        assert_eq!(p.state.action.as_ref().unwrap().id, "rest", "前提：八点半闲着");
        // 九点是上班时间。以前这里会一直发呆到某个数值见底
        let later = run(&c, p, 40, 8.5);
        assert_eq!(later.state.activity, Activity::Working, "九点了还在发呆");
    }

    #[test]
    fn 让她休息不是让她永远休息() {
        let c = cat();
        let mut p = run(&c, Pet::default(), 2, 10.0);
        p.state.feeling = 90.0;
        p = ask(&c, &p, "rest", 0.0);
        assert_eq!(p.state.action.as_ref().unwrap().id, "rest");
        assert_eq!(p.directive.as_ref().unwrap().left, DEFAULT_DIRECTIVE_MIN, "发呆不限时，得有个默认期限");
        let later = run(&c, p, (DEFAULT_DIRECTIVE_MIN as u32) + 5, 10.05);
        assert!(later.directive.is_none());
        assert_eq!(later.state.activity, Activity::Working, "歇够了就该回去上班");
    }

    #[test]
    fn 用户说了多久就多久() {
        let c = cat();
        let mut p = run(&c, Pet::default(), 2, 10.0);
        p.state.feeling = 90.0;
        p = ask_for(&c, &p, "play", 10.0);
        assert_eq!(p.directive.as_ref().unwrap().left, 10.0);
        assert_eq!(p.last_verdict.as_ref().unwrap().minutes, Some(10.0), "判定结果要带上期限，好告诉模型");
        let later = run(&c, p, 12, 10.05);
        assert!(later.directive.is_none());
        // 「玩一整天」也有上限
        let mut q = run(&c, Pet::default(), 2, 10.0);
        q.state.feeling = 90.0;
        q = ask_for(&c, &q, "play", 9999.0);
        assert!(q.directive.as_ref().unwrap().left <= MAX_REQUESTED_MIN);
    }

    #[test]
    fn 保护期里做完一轮会接着做同类的事() {
        let c = cat();
        let mut p = run(&c, Pet::default(), 2, 10.0);
        p.state.feeling = 90.0;
        p = ask_for(&c, &p, "play", 80.0); // play_game 一轮 30 分钟
        let first = p.state.action.as_ref().unwrap().id.clone();
        let later = run(&c, p, 45, 10.05);
        assert_eq!(later.state.activity, Activity::Playing, "一轮做完保护期还没过，该接着玩");
        assert_eq!(later.state.action.as_ref().unwrap().id, first);
    }

    #[test]
    fn 保护期压不过生理急需() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 90.0;
        p = ask_for(&c, &p, "play", 60.0);
        p.state.hunger = 3.0;
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
        assert_eq!(p.state.activity, Activity::Eating, "说了去玩也不能饿着");
        // 吃完还在保护期里，回去接着玩
        let later = run(&c, p, 5, 10.05);
        assert_eq!(later.state.activity, Activity::Playing, "吃完了该回去接着做你让做的事");
    }

    #[test]
    fn 拒绝了就没有保护期() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 30.0;
        let n = ask(&c, &p, "work", 1.0);
        assert!(!n.last_verdict.as_ref().unwrap().obey);
        assert!(n.directive.is_none());
        assert_eq!(n.last_verdict.as_ref().unwrap().minutes, None);
    }

    #[test]
    fn 保护期排在番茄钟前面() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 90.0;
        p = reduce(&c, &shelf(), &p, &Event::Pin(Some("work".into())));
        assert_eq!(p.state.activity, Activity::Working);
        p = ask_for(&c, &p, "play", 15.0);
        assert_eq!(p.state.activity, Activity::Playing, "你刚说的话比你之前开的番茄钟新");
        let later = run(&c, p, 10, 10.0);
        assert_eq!(later.state.activity, Activity::Playing);
        let after = run(&c, later, 10, 10.2);
        assert_eq!(after.state.activity, Activity::Working, "保护期过了番茄钟接管");
    }

    #[test]
    fn 请求可以用_tag_也可以用动作_id() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 90.0;
        let by_tag = ask(&c, &p, "work", 0.0);
        let by_id = ask(&c, &p, "work_copy", 0.0);
        assert_eq!(by_tag.state.activity, Activity::Working);
        assert_eq!(by_id.state.action.as_ref().unwrap().id, "work_copy");
    }

    #[test]
    fn 她照做的时候_reason_说明是你让的() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 95.0;
        let n = ask(&c, &p, "rest", 0.0);
        assert_eq!(n.state.action.as_ref().unwrap().reason, "你让我做的");
        assert!(n.last_verdict.as_ref().unwrap().obey);
    }

    #[test]
    fn 她拒绝的时候什么都不变() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 30.0; // 有点饿，但还够得着 work 的门槛
        let before = p.state.clone();
        let n = ask(&c, &p, "work", 1.0);
        let v = n.last_verdict.as_ref().unwrap();
        assert!(!v.obey);
        assert_eq!(n.state.activity, before.activity, "拒绝不该改变她在做的事");
        assert!(!v.say.is_empty(), "拒绝要有话说");
    }

    #[test]
    fn 判定结果只活一拍() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 95.0;
        let n = ask(&c, &p, "rest", 0.0);
        assert!(n.last_verdict.is_some());
        let later = reduce(&c, &shelf(), &n, &Event::Tick { minutes: 1.0, hour: 10.0 });
        assert!(later.last_verdict.is_none(), "不清掉的话同一句话会发一整天");
    }

    #[test]
    fn 用户开口能越过冷却() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 95.0;
        p.cooldowns.insert("work_copy".into(), 30.0);
        let n = ask(&c, &p, "work_copy", 0.0);
        assert!(n.state.action.as_ref().is_some_and(|a| a.id == "work_copy"));
        assert!(!n.cooldowns.contains_key("work_copy"));
    }

    #[test]
    fn 反复逼她会掉好感和心情() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 25.0;
        let aff0 = p.state.affection;
        // 连着要求四次，每次都掷出必拒
        for _ in 0..4 {
            p = ask(&c, &p, "work", 1.0);
        }
        assert!(p.state.affection < aff0, "被逼了四次好感还没掉");
        assert!(p.pressure >= 3.0);
        // 压力会自己散：一次要半小时，逼了四次就得两小时
        assert!(run(&c, p.clone(), 40, 10.0).pressure > 0.0, "才 40 分钟不该全消");
        assert_eq!(run(&c, p.clone(), 130, 10.0).pressure, 0.0);
    }

    #[test]
    fn 第一次拒绝不收费() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 25.0;
        let aff0 = p.state.affection;
        let n = ask(&c, &p, "work", 1.0);
        assert!(!n.last_verdict.as_ref().unwrap().obey);
        assert_eq!(n.state.affection, aff0, "拒绝一次不该有代价——那是她的权利");
    }

    #[test]
    fn 摸头涨好感但有额度() {
        let c = cat();
        let mut p = Pet::default();
        let aff0 = p.state.affection;
        for _ in 0..100 {
            p = reduce(&c, &shelf(), &p, &Event::Touched(Touch::Head));
        }
        let gained = p.state.affection - aff0;
        assert!(gained > 0.0);
        assert!(gained <= 12.0 * AFFECTION_HEAD + 1e-3, "连点一百下涨了 {gained}");
        assert_eq!(p.touch_budget, 0.0);
    }

    #[test]
    fn 抚摸额度会随时间回来() {
        let c = cat();
        let mut p = Pet::default();
        for _ in 0..20 {
            p = reduce(&c, &shelf(), &p, &Event::Touched(Touch::Head));
        }
        assert_eq!(p.touch_budget, 0.0);
        let later = run(&c, p, 60, 10.0);
        assert!(later.touch_budget > 5.0, "一小时后还是 {}", later.touch_budget);
    }

    #[test]
    fn 送礼涨好感() {
        let c = cat();
        let sh = stocked();
        let p = Pet::default();
        let n = reduce(&c, &sh, &p, &Event::Gifted { id: "cake".into(), name: "蛋糕".into() });
        assert!(n.state.affection > p.state.affection);
    }

    #[test]
    fn 好感度长期向基准线回归() {
        let c = cat();
        let mut p = Pet::default();
        p.state.affection = 100.0;
        // 七天不互动（只走时间），应当掉到 (100+40)/2 = 70 附近
        let later = run(&c, p, 0, 10.0);
        let mut q = later;
        for _ in 0..(7 * 24) {
            q = reduce(&c, &shelf(), &q, &Event::Tick { minutes: 60.0, hour: 10.0 });
        }
        assert!((q.state.affection - 70.0).abs() < 3.0, "七天后 {}", q.state.affection);
        assert!(q.state.affection > AFFECTION_BASELINE, "回归不是清零");
    }

    #[test]
    fn 好感度不会越界() {
        let c = cat();
        let mut p = Pet::default();
        p.state.affection = 99.9;
        for _ in 0..50 {
            p = reduce(&c, &shelf(), &p, &Event::Touched(Touch::Head));
        }
        assert!(p.state.affection <= 100.0);
    }

    #[test]
    fn 不认识的事她会说不会() {
        let c = cat();
        let p = Pet::default();
        let n = ask(&c, &p, "开飞机", 0.0);
        assert_eq!(n.last_verdict.as_ref().unwrap().refusal, Some(Refusal::Unknown));
    }

    #[test]
    fn 新宠物的抚摸额度是满的() {
        // Pet::default() 走的是手写 impl，不是 derive——serde default 管不到它
        assert_eq!(Pet::default().touch_budget, TOUCH_BUDGET_MAX);
        assert_eq!(Pet::default().state.affection, 50.0);
    }

    /* --- 健康 --- */

    /// 一颗药：health 40，体力 +20，价格 50
    fn pharmacy() -> FoodShelf {
        use super::super::food::FoodItem;
        let mut s = FoodShelf::default();
        s.set(vec![FoodItem {
            id: "pill".into(),
            name: "pill".into(),
            graph: "eat".into(),
            kind: "Drug".into(),
            strength: 20.0,
            strength_food: 0.0,
            strength_drink: 0.0,
            feeling: 0.0,
            health: 40.0,
            price: 50.0,
        }]);
        s
    }

    /// 只推数值，不让她换事做：直接喂 Tick 但把三项都按住
    fn hold(p: &mut Pet, hunger: f32, thirst: f32, strength: f32) {
        p.state.hunger = hunger;
        p.state.thirst = thirst;
        p.state.strength = strength;
    }

    #[test]
    fn 饿着渴着累着健康和心情都往下掉() {
        // 三项压在 UNWELL 线的两侧各过一小时。凌晨三点两边都在睡觉（作息压过「有点饿」），
        // 睡觉本身涨的那点心情两边一样，差出来的就是饿着渴着累着掉的
        let c = cat();
        let mut low = Pet::default();
        let mut fine = Pet::default();
        low.state.health = 80.0; // 满血会被 clamp 吃掉自然回的那点
        fine.state.health = 80.0;
        for _ in 0..60 {
            // 睡觉每分钟回 1 体力，按在线下两格才能整分钟都算「累着」
            hold(&mut low, UNWELL - 1.0, UNWELL - 1.0, UNWELL - 2.0);
            hold(&mut fine, UNWELL + 1.0, UNWELL + 1.0, UNWELL + 1.0);
            low = reduce(&c, &shelf(), &low, &Event::Tick { minutes: 1.0, hour: 3.0 });
            fine = reduce(&c, &shelf(), &fine, &Event::Tick { minutes: 1.0, hour: 3.0 });
        }
        assert_eq!(low.state.activity, Activity::Sleeping);
        assert_eq!(fine.state.activity, Activity::Sleeping);
        let lost = fine.state.health - low.state.health;
        let expected = HEALTH_LOSS * 3.0 * 60.0 + HEALTH_REGEN * 60.0;
        assert!((lost - expected).abs() < 0.5, "一小时该差 {expected}，差了 {lost}");
        let gap = fine.state.feeling - low.state.feeling;
        assert!((gap - FEELING_UNWELL * 3.0 * 60.0).abs() < 0.5, "饿着渴着累着心情该多掉 9：差了 {gap}");
    }

    #[test]
    fn 什么都不缺时健康慢慢养回来_病了就养不回来() {
        let c = cat();
        let mut p = Pet::default();
        p.state.health = 70.0;
        for _ in 0..120 {
            hold(&mut p, 90.0, 90.0, 90.0);
            p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 3.0 });
        }
        assert!(p.state.health > 70.0, "两小时没养回来一点：{}", p.state.health);
        assert!(p.state.health < 80.0, "养得太快了：{}", p.state.health);

        // 病着 vs 没病，其余一样过一小时：差的那截就是病掉的心情（她在做的事本身也涨跌心情）
        let mut sick = Pet::default();
        sick.state.health = 40.0; // 病线以下
        let mut well = Pet::default();
        for _ in 0..60 {
            hold(&mut sick, 90.0, 90.0, 90.0);
            hold(&mut well, 90.0, 90.0, 90.0);
            sick = reduce(&c, &shelf(), &sick, &Event::Tick { minutes: 1.0, hour: 3.0 });
            well = reduce(&c, &shelf(), &well, &Event::Tick { minutes: 1.0, hour: 3.0 });
        }
        assert!(sick.state.health <= 40.0, "病了不该自己好：{}", sick.state.health);
        let gap = well.state.feeling - sick.state.feeling;
        assert!(gap >= FEELING_SICK * 60.0 - 0.5, "病着心情该大幅掉：没病 {} 病着 {}", well.state.feeling, sick.state.feeling);
    }

    #[test]
    fn 喂药之后健康慢慢好转_不是一口见底() {
        let c = cat();
        let ph = pharmacy();
        let mut p = Pet::default();
        p.state.health = 40.0;
        p.state.strength = 50.0;
        let aff = p.state.affection;
        p = reduce(&c, &ph, &p, &Event::Medicated { id: "pill".into(), name: "pill".into() });
        assert_eq!(p.state.action.as_ref().map(|a| a.id.as_str()), Some("medicine"));
        assert_eq!(p.state.activity, Activity::Eating, "借吃的夹心动画");
        assert_eq!(p.state.action.as_ref().and_then(|a| a.food.as_ref()).map(|f| f.id.as_str()), Some("pill"));
        assert!((p.state.health - 40.0).abs() < 0.01, "药效不该当场到账：{}", p.state.health);
        assert_eq!(p.state.remedy, 40.0);
        assert_eq!(p.state.strength, 70.0, "药自带的体力当场给");
        assert!(p.state.affection > aff, "病中喂药该涨好感");
        // 十分钟后吸收了 REMEDY_PER_MIN × 10
        for _ in 0..10 {
            hold(&mut p, 90.0, 90.0, 90.0);
            p = reduce(&c, &ph, &p, &Event::Tick { minutes: 1.0, hour: 3.0 });
        }
        let gained = p.state.health - 40.0;
        assert!((gained - REMEDY_PER_MIN * 10.0).abs() < 0.5, "十分钟该回 {}，回了 {gained}", REMEDY_PER_MIN * 10.0);
        assert!(p.state.remedy < 40.0 && p.state.remedy > 0.0, "药效池该在慢慢消耗：{}", p.state.remedy);
        // 吸收完就停在应到的量
        for _ in 0..120 {
            hold(&mut p, 90.0, 90.0, 90.0);
            p = reduce(&c, &ph, &p, &Event::Tick { minutes: 1.0, hour: 3.0 });
        }
        assert_eq!(p.state.remedy, 0.0);
        assert!(p.state.health >= 80.0 - 0.5, "40 + 40 的药该到 80 上下：{}", p.state.health);
    }

    #[test]
    fn 健康决定四套动画里的哪一套() {
        let mut s = PetState::default();
        s.feeling = 90.0;
        s.health = 100.0;
        assert_eq!(derive_mood(&s), Mood::Happy);
        s.health = 45.0;
        assert_eq!(derive_mood(&s), Mood::Nomal, "病着再开心也只到 Nomal");
        s.health = 20.0;
        assert_eq!(derive_mood(&s), Mood::Ill);
        s.health = 100.0;
        s.feeling = 30.0;
        assert_eq!(derive_mood(&s), Mood::PoorCondition);
    }

    #[test]
    fn 病重了正事一律放下() {
        let c = cat();
        let mut p = run(&c, Pet::default(), 30, 10.0);
        assert_eq!(p.state.activity, Activity::Working, "前提：十点在上班");
        p = reduce(&c, &shelf(), &p, &Event::Patch {
            strength: None, feeling: None, hunger: None, thirst: None, money: None, affection: None,
            health: Some(15.0),
        });
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.5 });
        assert_eq!(p.state.mood, Mood::Ill);
        let a = p.state.action.as_ref().unwrap();
        assert!(!SUPPRESSIBLE.iter().any(|t| c.get(&a.id).unwrap().has_tag(t)), "病重了还在 {}", a.name);
        // 使唤她去工作：不掷骰子，直接说不舒服
        p = reduce(&c, &shelf(), &p, &Event::Request { target: "work".into(), roll: 0.0, minutes: None });
        let v = p.last_verdict.clone().unwrap();
        assert!(!v.obey);
        assert_eq!(v.refusal, Some(Refusal::Sick));
        // 但吃饭照吃：病人也要吃饭
        p.state.hunger = 5.0;
        p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour: 10.6 });
        assert_eq!(p.state.activity, Activity::Eating);
    }

    #[test]
    fn 旧存档没有健康字段时按满血读() {
        let json = serde_json::to_string(&PetState::default()).unwrap().replace("\"health\":100.0,", "").replace("\"remedy\":0.0,", "");
        assert!(!json.contains("health"), "{json}");
        let s: PetState = serde_json::from_str(&json).unwrap();
        assert_eq!(s.health, 100.0);
        assert_eq!(s.remedy, 0.0);
    }
}


#[cfg(test)]
mod 一天的生活 {
    use super::*;

    fn shelf() -> FoodShelf {
        FoodShelf::default()
    }

    #[test]
    #[ignore = "不是断言，是拿来看她一天怎么过的"]
    fn 打印作息表() {
        let c = Catalog::load();
        let mut p = Pet::default();
        let mut last = String::new();
        for i in 0..(24 * 60) {
            let hour = i as f32 / 60.0;
            p = reduce(&c, &shelf(), &p, &Event::Tick { minutes: 1.0, hour });
            let now = p.state.action.as_ref().map(|a| a.id.clone()).unwrap_or_default();
            if now != last {
                let a = p.state.action.as_ref().unwrap();
                println!(
                    "{:02}:{:02}  {:<6} {:<12} 体力{:3.0} 心情{:3.0} 饱腹{:3.0} 水{:3.0} 钱{:5.0} Lv{}",
                    i / 60, i % 60, a.name, a.reason,
                    p.state.strength, p.state.feeling, p.state.hunger, p.state.thirst,
                    p.state.money, p.state.level
                );
                last = now;
            }
        }
    }
}
