//! 主动开口（Nudge Policy）：她什么时候、以什么分量、对谁说一句话。全是确定性规则，
//! 模型只管「怎么说」（`lines::say_in_character` / `lines::mumble`），不管「说不说」。
//!
//! 「不突兀」的核心是每次开口都有用户看得见的由头。四种由头：
//!
//! | 由头 | 例子 | 分量 |
//! |---|---|---|
//! | 你的事到点了 | 早上一句今日概览、每件事开始前一句「该开始了」 | 搭话 |
//! | 你回来了 | 离开半小时以上又坐下：接一句待办 / 她这段时间干了什么 | 搭话 |
//! | 你坐太久了 | 连续两小时没停：先自言自语一句，再半小时还没停就搭话 | 自言自语 → 搭话 |
//! | 她自己的日子 | 隔一阵子嘀咕一句（模型写，只在内存不紧时） | 自言自语 |
//!
//! 两种分量：**搭话**（对你说，占每日预算，出声）和**自言自语**（不对你说，不占预算，
//! 轻声——音量减半）。预算每天 `DAILY_BUDGET` 次，按昨天的反应调：你回了话就多给一点，
//! 连着几次没理就少给；「别烦我」当天清零，这是规则，不经模型。
//!
//! 每条话有 key，说过就不再说——**换个措辞也不说第二次**。所有账本按天记在 `kv`，重启不重复。
//! 「人不在就闭嘴」由调用方（lib.rs）把关：夜里、她睡着、你不在电脑前、刚从待机醒来都不来问。

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::kb::{Agenda, KbEvent};

/// 开始前多少分钟提醒
pub const DEFAULT_LEAD_MIN: u32 = 15;
/// 一天主动搭话的预算（次）。自言自语不算
pub const DAILY_BUDGET: u32 = 6;
/// 预算按昨天的反应调整后的上下限
pub const BUDGET_MIN: u32 = 2;
pub const BUDGET_MAX: u32 = 8;
/// 昨天回了几次话，今天就多几次（最多加这么多）
pub const REPLY_BONUS_MAX: u32 = 2;
/// 昨天连着这么多次没理她，今天少两次
pub const IGNORED_STREAK_PENALTY_AT: u32 = 3;
pub const IGNORED_PENALTY: u32 = 2;
/// 搭话之后多久内有回应算「回了话」（分钟）
pub const REPLY_WINDOW_MIN: f32 = 10.0;
/// 两条搭话至少隔多久（分钟）。「该开始了」有时效，可以近一点
pub const MIN_GAP_MIN: f32 = 30.0;
pub const MIN_GAP_SOON_MIN: f32 = 5.0;
/// 今日概览最早 / 最晚几点说。8 点她才起；中午以后才开机就直接等「该开始了」
pub const BRIEF_FROM_HOUR: u32 = 8;
pub const BRIEF_UNTIL_HOUR: u32 = 14;
/// 你多久没动键鼠就算不在（秒）
pub const DEFAULT_IDLE_MAX_SEC: u32 = 300;
/// 从待机醒来后先安静这么久（分钟）
pub const WAKE_GRACE_MIN: f32 = 3.0;
/// 离开多久算「走了」（分钟）；回来后等多久再开口；一天最多几次「你回来啦」
pub const AWAY_MIN: f32 = 30.0;
pub const BACK_DELAY_MIN: f32 = 1.5;
pub const BACK_WINDOW_MIN: f32 = 10.0;
pub const BACK_MAX_PER_DAY: u32 = 3;
/// 连续在电脑前多久提一句歇（分钟）；再过多久还没停就搭话
pub const REST_STREAK_MIN: f32 = 120.0;
pub const REST_ESCALATE_MIN: f32 = 30.0;
/// 自言自语的间隔区间（分钟）——不均匀才不像闹钟
pub const MUMBLE_GAP_MIN: (f32, f32) = (5.0, 6.0);
/// 内存占用超过这个百分比就不让模型嘀咕（对话模型要占好几 GB）
pub const MEMORY_LOAD_MAX: u32 = 85;
/// 自言自语 / 「歇一下」的音量（0–1）
pub const QUIET_VOLUME: f32 = 0.5;
/// 标题太长就截：气泡只有几行
const TITLE_MAX_CHARS: usize = 24;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct NudgeSettings {
    pub enabled: bool,
    /// 开始前多少分钟提醒
    pub lead_min: u32,
    /// 多久没动键鼠算不在（秒）
    pub idle_max_sec: u32,
    /// 一天主动搭话几次（自言自语不算）
    pub daily_budget: u32,
    /// 让模型隔一阵子嘀咕一句
    pub self_talk: bool,
    /// 自言自语 / 关心你歇一下的音量
    pub quiet_volume: f32,
}

impl Default for NudgeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            lead_min: DEFAULT_LEAD_MIN,
            idle_max_sec: DEFAULT_IDLE_MAX_SEC,
            daily_budget: DAILY_BUDGET,
            self_talk: true,
            quiet_volume: QUIET_VOLUME,
        }
    }
}

/// 分量：对你说，还是自己嘀咕
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// 搭话：对你说，占预算，正常音量
    Talk,
    /// 自言自语：不对你说，不占预算，轻声
    SelfTalk,
}

/// 一条要说的话
#[derive(Debug, Clone, PartialEq)]
pub struct Nudge {
    pub key: String,
    pub text: String,
    pub level: Level,
    /// 顺手写进她记忆的一句（只有今日概览有）
    pub remember: Option<String>,
}

/// 一天的账本：说过什么、用了几次预算、静音到几点、你的反应。整个按天存在 `kv`
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DayBook {
    pub date: String,
    pub said: HashSet<String>,
    /// 今天的预算和已用
    pub limit: u32,
    pub used: u32,
    /// 「别烦我」：到这个时刻（ms）之前一个字不说
    pub muted_until_ms: i64,
    /// 你的反应：搭话后 10 分钟内回了话 / 没理
    pub replied: u32,
    pub ignored_streak: u32,
    /// 上一条搭话：key 和时间，等你的反应
    pub awaiting: Option<(String, i64)>,
    pub last_talk_ms: Option<i64>,
    pub back_count: u32,
    /// 今天嘀咕过的话，给模型「别重复」
    pub mumbles: VecDeque<String>,
    /// 成功嘀咕的次数；让模型在同一种活动里轮换观察角度。
    pub mumble_seq: u32,
    pub last_mumble_ms: Option<i64>,
}

impl DayBook {
    /// 新的一天：预算按昨天的反应调
    pub fn open(date: &str, base_budget: u32, yesterday: Option<&DayBook>) -> Self {
        let limit = match yesterday {
            Some(y) => {
                let bonus = y.replied.min(REPLY_BONUS_MAX);
                let penalty = if y.ignored_streak >= IGNORED_STREAK_PENALTY_AT { IGNORED_PENALTY } else { 0 };
                (base_budget + bonus).saturating_sub(penalty).clamp(BUDGET_MIN, BUDGET_MAX)
            }
            None => base_budget.clamp(BUDGET_MIN, BUDGET_MAX),
        };
        Self { date: date.into(), limit, ..Default::default() }
    }

    pub fn muted(&self, now_ms: i64) -> bool {
        now_ms < self.muted_until_ms
    }

    pub fn budget_left(&self) -> u32 {
        self.limit.saturating_sub(self.used)
    }

    /// 「别烦我」：`until_ms` 之前不搭话；不给时长就是今天剩下的时间
    pub fn mute(&mut self, until_ms: i64) {
        self.muted_until_ms = self.muted_until_ms.max(until_ms);
        self.awaiting = None;
    }

    /// 说了一条。搭话占预算并开始等你的反应；自言自语只记 key
    pub fn record(&mut self, n: &Nudge, now_ms: i64) {
        self.said.insert(n.key.clone());
        if n.level == Level::Talk {
            self.used += 1;
            self.last_talk_ms = Some(now_ms);
            self.awaiting = Some((n.key.clone(), now_ms));
            if n.key.starts_with("back:") {
                self.back_count += 1;
            }
        }
    }

    /// 你有了动作（发消息、摸她）：上一条搭话算被回应了
    pub fn user_reacted(&mut self, now_ms: i64) {
        if let Some((_, at)) = self.awaiting.take() {
            if (now_ms - at) as f32 / 60_000.0 <= REPLY_WINDOW_MIN {
                self.replied += 1;
                self.ignored_streak = 0;
            } else {
                self.ignored_streak += 1;
            }
        }
    }

    /// 等回应的窗口过了还没动静：算没理
    pub fn settle(&mut self, now_ms: i64) {
        if let Some((_, at)) = self.awaiting {
            if (now_ms - at) as f32 / 60_000.0 > REPLY_WINDOW_MIN {
                self.awaiting = None;
                self.ignored_streak += 1;
            }
        }
    }

    pub fn note_mumble(&mut self, text: &str, now_ms: i64) {
        self.mumbles.push_back(text.to_string());
        while self.mumbles.len() > 6 {
            self.mumbles.pop_front();
        }
        self.mumble_seq = self.mumble_seq.wrapping_add(1);
        self.last_mumble_ms = Some(now_ms);
    }
}

/// 此刻的处境：你在干嘛、她在干嘛。全部由 lib.rs 从本机信号算出来，这里只做判断
#[derive(Debug, Clone, Default)]
pub struct Moment {
    pub now_ms: i64,
    /// 从零点起的分钟
    pub minute_of_day: u32,
    /// 你刚回来：离开了多久（分钟）；不在「刚回来」的窗口里就是 None
    pub just_back_after_min: Option<f32>,
    /// 回来之后过了多久（分钟）
    pub back_for_min: f32,
    /// 你连续在电脑前多久（分钟）
    pub active_streak_min: f32,
    /// 番茄钟 / 专注段在跑（她已经在管你的节奏了，不用再提歇）
    pub focus_running: bool,
    /// 你不在的时候她做过的事（动作名，按时间）
    pub her_recent: Vec<String>,
    /// 内存占用百分比
    pub memory_load: u32,
}

/// 现在该不该开口、说什么。纯函数：处境 + 日程 + 账本 进，一条话 出
pub fn pick(agenda: &Agenda, m: &Moment, book: &DayBook, lead_min: u32) -> Option<Nudge> {
    if book.muted(m.now_ms) {
        return None;
    }
    let since_last = book.last_talk_ms.map(|t| (m.now_ms - t) as f32 / 60_000.0);
    let gap_ok = |need: f32| since_last.map_or(true, |g| g >= need);
    let can_talk = |need: f32| book.budget_left() > 0 && gap_ok(need);

    // 1. 马上要开始的事：时效最高
    if can_talk(MIN_GAP_SOON_MIN) {
        for e in agenda.pending() {
            let Some(start) = e.start else { continue };
            let key = format!("soon:{}", e.id);
            let in_window = start <= m.minute_of_day + lead_min && m.minute_of_day <= start + 1;
            if in_window && !book.said.contains(&key) {
                return Some(Nudge {
                    key,
                    text: format!("{} 该开始「{}」了。", clock(start), short(&e.title)),
                    level: Level::Talk,
                    remember: None,
                });
            }
        }
    }

    // 2. 你回来了：等一会儿再开口，接一句最有用的
    if let Some(away) = m.just_back_after_min {
        let key = key_for_back(m);
        if away >= AWAY_MIN
            && m.back_for_min >= BACK_DELAY_MIN
            && book.back_count < BACK_MAX_PER_DAY
            && !book.said.contains(&key)
            && can_talk(MIN_GAP_SOON_MIN)
        {
            if let Some(text) = welcome_back(agenda, m, away) {
                return Some(Nudge { key, text, level: Level::Talk, remember: None });
            }
        }
    }

    // 3. 今日概览：早上第一次看到你
    let hour = m.minute_of_day / 60;
    let key = format!("brief:{}", agenda.date);
    if (BRIEF_FROM_HOUR..BRIEF_UNTIL_HOUR).contains(&hour) && !book.said.contains(&key) && can_talk(MIN_GAP_MIN) {
        if let Some((text, remember)) = brief(agenda) {
            return Some(Nudge { key, text, level: Level::Talk, remember: Some(remember) });
        }
    }

    // 4. 坐太久了：先自己嘀咕一句（不占预算），再半小时还没停才搭话
    if !m.focus_running && m.active_streak_min >= REST_STREAK_MIN {
        // 这一段连续活跃从哪一刻开始（按半小时取整），同一段只提一次
        let episode = (m.now_ms / 60_000 - m.active_streak_min as i64) / 30;
        let k1 = format!("rest:{episode}:1");
        if !book.said.contains(&k1) {
            return Some(Nudge {
                key: k1,
                text: format!("……坐了{}了。", hours_word(m.active_streak_min)),
                level: Level::SelfTalk,
                remember: None,
            });
        }
        let k2 = format!("rest:{episode}:2");
        if m.active_streak_min >= REST_STREAK_MIN + REST_ESCALATE_MIN && !book.said.contains(&k2) && can_talk(MIN_GAP_SOON_MIN) {
            return Some(Nudge {
                key: k2,
                text: format!("你已经连着坐了{}了，起来走两步再回来吧。", hours_word(m.active_streak_min)),
                level: Level::Talk,
                remember: None,
            });
        }
    }
    None
}

/// 「你回来啦」的 key 按回来的那一刻算（五分钟取整），同一次回来只说一次
fn key_for_back(m: &Moment) -> String {
    let returned_at_min = m.now_ms / 60_000 - m.back_for_min as i64;
    format!("back:{}", returned_at_min / 5 * 5)
}

/// 你回来了接哪句：待办的日程 > 她这段时间干了什么 > 什么都没有就不开口
fn welcome_back(agenda: &Agenda, m: &Moment, away: f32) -> Option<String> {
    let next = agenda.pending().find(|e| e.start.map_or(false, |s| s + 1 >= m.minute_of_day));
    if let Some(e) = next {
        let when = e.start.map(clock).unwrap_or_default();
        return Some(format!("你回来啦。{} 还有「{}」。", when, short(&e.title)));
    }
    if !m.her_recent.is_empty() {
        let did: Vec<&str> = m.her_recent.iter().map(String::as_str).collect();
        return Some(format!("你回来啦。你不在的{}里我{}了。", hours_word(away), did.join("、")));
    }
    None
}

/// 自言自语该不该到点了（模型写的那种）。间隔不均匀，用当天的分钟数掺一点抖动
pub fn mumble_due(book: &DayBook, m: &Moment, enabled: bool) -> bool {
    if !enabled || book.muted(m.now_ms) || m.memory_load > MEMORY_LOAD_MAX {
        return false;
    }
    let (lo, hi) = MUMBLE_GAP_MIN;
    let jitter = (m.now_ms / 60_000 % 7) as f32 / 6.0; // 0–1
    let gap = lo + (hi - lo) * jitter;
    match book.last_mumble_ms {
        None => m.back_for_min >= BACK_DELAY_MIN,
        Some(t) => (m.now_ms - t) as f32 / 60_000.0 >= gap,
    }
}

/// 「别烦我」「安静一小时」：返回静音多少分钟（None = 今天剩下的时间）。
/// 认不出来就是外层的 None——这是命令，宁可漏也别把闲聊当命令
pub fn parse_quiet(text: &str) -> Option<Option<f32>> {
    let t: String = text
        .trim()
        .chars()
        .filter(|c| !matches!(c, '，' | '。' | '！' | '～' | ',' | '.' | '!' | '~'))
        .collect();
    if t.chars().count() > 20 {
        return None;
    }
    let lower = t.to_lowercase();
    // 「安静」单独不算——「今天好安静啊」是闲聊；带「点 / 一下 / 会」或者开头就是「安静」才是命令
    const ZH: [&str; 13] = [
        "别烦我", "别吵", "别说话", "让我静静", "闭嘴", "别打扰", "不要打扰", "别来烦", "别理我",
        "安静点", "安静一", "安静会", "保持安静",
    ];
    const EN: [&str; 4] = ["shut up", "be quiet", "leave me alone", "don't disturb"];
    let zh_hit = ZH.iter().any(|k| t.contains(k)) || t.starts_with("安静");
    if !zh_hit && !EN.iter().any(|k| lower.contains(k)) {
        return None;
    }
    // 「安静一小时」「别烦我半小时」
    let (minutes, _) = crate::core::intent::duration_zh(&t);
    Some(minutes)
}

/// 「可以说话了」「不用安静了」：解除静音
pub fn parse_unquiet(text: &str) -> bool {
    let t: String = text.trim().chars().filter(|c| !matches!(c, '，' | '。' | '！' | '～' | ',' | '.' | '!' | '~')).collect();
    if t.chars().count() > 16 {
        return false;
    }
    const ZH: [&str; 6] = ["可以说话了", "不用安静了", "解除静音", "说话吧", "可以吵我了", "不烦了"];
    ZH.iter().any(|k| t.contains(k)) || t.to_lowercase().contains("you can talk")
}

/// 你主动问「今天有什么安排」时用：不看钟点、不看说没说过，直接给概览
pub fn brief_now(agenda: &Agenda) -> Option<Nudge> {
    brief(agenda).map(|(text, remember)| Nudge {
        key: format!("brief:{}", agenda.date),
        text,
        level: Level::Talk,
        remember: Some(remember),
    })
}

/// 今日概览的一句话，以及写进记忆的那句。今天什么都没有就不说（None）——没事也开口才是吵
fn brief(agenda: &Agenda) -> Option<(String, String)> {
    let pending: Vec<&KbEvent> = agenda.pending().collect();
    let mut parts: Vec<String> = Vec::new();
    match pending.as_slice() {
        [] => {}
        [one] => parts.push(match one.start {
            Some(s) => format!("今天有一件事：{} 的「{}」", clock(s), short(&one.title)),
            None => format!("今天有一件事：「{}」", short(&one.title)),
        }),
        [first, ..] => parts.push(match first.start {
            Some(s) => format!("今天排了 {} 件事，{} 从「{}」开始", pending.len(), clock(s), short(&first.title)),
            None => format!("今天排了 {} 件事，先是「{}」", pending.len(), short(&first.title)),
        }),
    }
    if agenda.reviews_due > 0 {
        parts.push(format!("有 {} 篇笔记到期该复习了", agenda.reviews_due));
    }
    if parts.is_empty() {
        return None;
    }
    let text = format!("{}。", parts.join("；"));

    // 记忆里写全一点：她被问「今晚学什么」时要答得上来
    let list: Vec<String> = pending
        .iter()
        .map(|e| match (e.start, e.end) {
            (Some(s), Some(t)) => format!("{}–{} {}", clock(s), clock(t), e.title),
            (Some(s), None) => format!("{} {}", clock(s), e.title),
            _ => e.title.clone(),
        })
        .collect();
    let mut remember = format!("{} 的安排：{}", pretty_date(&agenda.date), if list.is_empty() { "没排事".into() } else { list.join("、") });
    if agenda.reviews_due > 0 {
        remember.push_str(&format!("；{} 篇笔记到期待复习", agenda.reviews_due));
    }
    Some((text, remember))
}

/// 给对话的「此刻」用：用户今天的安排一句话。知识库没装 / 今天没事就是 None。
/// 写明「用户的」——这是他的日程，不是她的
pub fn describe_agenda() -> Option<String> {
    let db = crate::kb::locate()?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let agenda = crate::kb::agenda(&db, &today).ok()?;
    let (_, remember) = brief(&agenda)?;
    Some(format!("用户{}（来自他的知识库，只能转述，不能替他改）。", remember))
}

fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// 「2个小时」「2个半小时」「快3个小时」「45分钟」
fn hours_word(minutes: f32) -> String {
    let m = minutes.round() as u32;
    if m < 60 {
        return format!("{m}分钟");
    }
    let h = m / 60;
    let rest = m % 60;
    if rest >= 45 {
        format!("快{}个小时", h + 1)
    } else if rest >= 20 {
        format!("{h}个半小时")
    } else {
        format!("{h}个小时")
    }
}

fn short(title: &str) -> String {
    let n = title.chars().count();
    if n <= TITLE_MAX_CHARS {
        title.to_string()
    } else {
        let cut: String = title.chars().take(TITLE_MAX_CHARS - 1).collect();
        format!("{cut}…")
    }
}

/// `2026-09-18` → `9月18日`
fn pretty_date(date: &str) -> String {
    let mut it = date.split('-').skip(1).filter_map(|p| p.parse::<u32>().ok());
    match (it.next(), it.next()) {
        (Some(m), Some(d)) => format!("{m}月{d}日"),
        _ => date.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 1_789_700_000_000; // 随便一个时刻，只用来算相对时间

    fn ev(id: &str, title: &str, start: Option<u32>, done: bool) -> KbEvent {
        KbEvent { id: id.into(), title: title.into(), start, end: start.map(|s| s + 60), kind: "study".into(), done }
    }

    fn day() -> Agenda {
        Agenda {
            date: "2026-09-18".into(),
            events: vec![
                ev("a", "高数 p43 42 有理函数的积分（上集）", Some(20 * 60), false),
                ev("b", "高数 p44 43 有理函数的积分（下集）", Some(21 * 60), false),
                ev("c", "高数 p45 44 定积分的定义", Some(22 * 60), false),
            ],
            reviews_due: 3,
        }
    }

    fn at(minute_of_day: u32) -> Moment {
        Moment { now_ms: DAY_MS + minute_of_day as i64 * 60_000, minute_of_day, back_for_min: 30.0, ..Default::default() }
    }

    fn book() -> DayBook {
        DayBook::open("2026-09-18", DAILY_BUDGET, None)
    }

    #[test]
    fn 早上一条概览_简短_并写记忆() {
        let n = pick(&day(), &at(9 * 60), &book(), 15).expect("早上该有概览");
        assert_eq!(n.key, "brief:2026-09-18");
        assert_eq!(n.level, Level::Talk);
        assert_eq!(n.text, "今天排了 3 件事，20:00 从「高数 p43 42 有理函数的积分（上集）」开始；有 3 篇笔记到期该复习了。");
        assert!(n.text.chars().count() < 60, "太长了：{}", n.text);
        let mem = n.remember.unwrap();
        assert!(mem.starts_with("9月18日 的安排：20:00–21:00 高数 p43"), "{mem}");
        assert!(mem.contains("3 篇笔记"));
    }

    #[test]
    fn 概览只说一次_夜里和下午不说() {
        let mut b = book();
        b.said.insert("brief:2026-09-18".into());
        assert_eq!(pick(&day(), &at(9 * 60), &b, 15), None, "说过了");
        assert_eq!(pick(&day(), &at(3 * 60), &book(), 15), None, "凌晨");
        assert_eq!(pick(&day(), &at(15 * 60), &book(), 15), None, "下午才开机就等「该开始了」");
    }

    #[test]
    fn 今天没事就闭嘴() {
        let a = Agenda { date: "2026-09-18".into(), events: vec![ev("x", "做完的", Some(600), true)], reviews_due: 0 };
        assert_eq!(pick(&a, &at(9 * 60), &book(), 15), None);
    }

    #[test]
    fn 开始前十五分钟提醒一次() {
        assert_eq!(pick(&day(), &at(19 * 60 + 30), &book(), 15), None, "还早");
        let n = pick(&day(), &at(19 * 60 + 46), &book(), 15).unwrap();
        assert_eq!(n.key, "soon:a");
        assert_eq!(n.text, "20:00 该开始「高数 p43 42 有理函数的积分（上集）」了。");
        assert!(n.remember.is_none());
        let mut b = book();
        b.said.insert("soon:a".into());
        assert_eq!(pick(&day(), &at(19 * 60 + 50), &b, 15), None, "说过了就不重复");
        assert_eq!(pick(&day(), &at(20 * 60 + 10), &book(), 15), None, "已经开始十分钟了，不追");
    }

    #[test]
    fn 两条之间要隔一会儿_但时效的可以近一点() {
        let mut b = book();
        b.said.insert("brief:2026-09-18".into());
        let m = at(19 * 60 + 50);
        b.last_talk_ms = Some(m.now_ms - 2 * 60_000);
        assert_eq!(pick(&day(), &m, &b, 15), None, "刚说过两分钟");
        b.last_talk_ms = Some(m.now_ms - 6 * 60_000);
        assert!(pick(&day(), &m, &b, 15).is_some(), "六分钟够了");
        let mut b = book();
        let m = at(9 * 60);
        b.last_talk_ms = Some(m.now_ms - 10 * 60_000);
        assert_eq!(pick(&day(), &m, &b, 15), None, "概览要隔半小时");
    }

    #[test]
    fn 预算用完就不搭话_但自言自语照旧() {
        let mut b = book();
        b.used = b.limit;
        assert_eq!(pick(&day(), &at(19 * 60 + 50), &b, 15), None, "预算没了连「该开始了」也不说");
        let mut m = at(19 * 60 + 50);
        m.active_streak_min = 130.0;
        let n = pick(&day(), &m, &b, 15).expect("坐太久的嘀咕不占预算");
        assert_eq!(n.level, Level::SelfTalk);
        assert!(n.key.starts_with("rest:") && n.key.ends_with(":1"));
    }

    #[test]
    fn 别烦我_今天清零_不经模型() {
        let mut b = book();
        let m = at(19 * 60 + 50);
        b.mute(m.now_ms + 3 * 3_600_000);
        assert_eq!(pick(&day(), &m, &b, 15), None);
        assert!(!mumble_due(&b, &m, true), "静音期间也不嘀咕");
        assert_eq!(parse_quiet("别烦我"), Some(None));
        assert_eq!(parse_quiet("安静一小时"), Some(Some(60.0)));
        assert_eq!(parse_quiet("别吵我半小时"), Some(Some(30.0)));
        assert_eq!(parse_quiet("leave me alone"), Some(None));
        assert_eq!(parse_quiet("今天好安静啊，一个人都没有，感觉有点冷清"), None, "闲聊不是命令");
        assert!(parse_unquiet("可以说话了"));
        assert!(!parse_unquiet("今天说话说得好累"));
    }

    #[test]
    fn 预算按昨天的反应调() {
        let mut y = book();
        y.replied = 3;
        assert_eq!(DayBook::open("2026-09-19", 6, Some(&y)).limit, 8, "回了话就多给，最多加两次");
        let mut y = book();
        y.ignored_streak = 3;
        assert_eq!(DayBook::open("2026-09-19", 6, Some(&y)).limit, 4, "连着三次没理，少两次");
        let mut y = book();
        y.ignored_streak = 5;
        assert_eq!(DayBook::open("2026-09-19", 2, Some(&y)).limit, 2, "有下限");
    }

    #[test]
    fn 回应窗口_十分钟内算回了话() {
        let mut b = book();
        let n = pick(&day(), &at(19 * 60 + 46), &b, 15).unwrap();
        let t = at(19 * 60 + 46).now_ms;
        b.record(&n, t);
        assert_eq!(b.used, 1);
        b.user_reacted(t + 3 * 60_000);
        assert_eq!((b.replied, b.ignored_streak), (1, 0));
        let n2 = Nudge { key: "x".into(), text: String::new(), level: Level::Talk, remember: None };
        b.record(&n2, t + 60 * 60_000);
        b.settle(t + 75 * 60_000);
        assert_eq!(b.ignored_streak, 1, "十五分钟没动静算没理");
        assert!(b.awaiting.is_none());
    }

    #[test]
    fn 你回来了_先接待办_没有就说她干了什么_都没有就不说() {
        let mut m = at(15 * 60);
        m.just_back_after_min = Some(45.0);
        m.back_for_min = 2.0;
        let n = pick(&day(), &m, &book(), 15).unwrap();
        assert!(n.key.starts_with("back:"));
        assert_eq!(n.text, "你回来啦。20:00 还有「高数 p43 42 有理函数的积分（上集）」。");
        // 没有待办：说她干了什么
        let empty = Agenda { date: "2026-09-18".into(), events: vec![], reviews_due: 0 };
        m.her_recent = vec!["清屏".into(), "写文案".into()];
        let n = pick(&empty, &m, &book(), 15).unwrap();
        assert_eq!(n.text, "你回来啦。你不在的45分钟里我清屏、写文案了。");
        // 什么都没有：不开口
        m.her_recent.clear();
        assert_eq!(pick(&empty, &m, &book(), 15), None);
        // 刚坐下不到一分半不说；离开不到半小时不说
        m.back_for_min = 0.5;
        assert_eq!(pick(&day(), &m, &book(), 15), None);
        m.back_for_min = 2.0;
        m.just_back_after_min = Some(10.0);
        assert_eq!(pick(&day(), &m, &book(), 15), None);
    }

    #[test]
    fn 坐太久_先嘀咕再搭话_番茄钟跑着不管() {
        let mut m = at(16 * 60);
        m.active_streak_min = 125.0;
        let mut b = book();
        b.said.insert("brief:2026-09-18".into());
        let n = pick(&day(), &m, &b, 15).unwrap();
        assert_eq!(n.level, Level::SelfTalk);
        assert_eq!(n.text, "……坐了2个小时了。");
        b.record(&n, m.now_ms);
        assert_eq!(b.used, 0, "自言自语不占预算");
        m.active_streak_min = 140.0;
        m.now_ms += 15 * 60_000;
        assert_eq!(pick(&day(), &m, &b, 15), None, "还没到升级的点");
        m.active_streak_min = 155.0;
        m.now_ms += 15 * 60_000;
        let n = pick(&day(), &m, &b, 15).unwrap();
        assert_eq!(n.level, Level::Talk);
        assert!(n.text.contains("起来走两步"), "{}", n.text);
        m.focus_running = true;
        let mut fresh = book();
        fresh.said.insert("brief:2026-09-18".into());
        assert_eq!(pick(&day(), &m, &fresh, 15), None, "番茄钟在跑就不提歇");
    }

    #[test]
    fn 嘀咕的间隔不均匀_内存紧了不嘀咕() {
        let b = book();
        let m = at(15 * 60);
        assert!(mumble_due(&b, &m, true), "刚坐下一会儿可以嘀咕第一句");
        assert!(!mumble_due(&b, &m, false));
        let mut m2 = m.clone();
        m2.memory_load = 90;
        assert!(!mumble_due(&b, &m2, true));
        let mut b2 = b.clone();
        b2.last_mumble_ms = Some(m.now_ms - 4 * 60_000);
        assert!(!mumble_due(&b2, &m, true), "四分钟前刚嘀咕过");
        b2.last_mumble_ms = Some(m.now_ms - 7 * 60_000);
        assert!(mumble_due(&b2, &m, true));
    }

    #[test]
    fn 嘀咕成功后推进角度序号() {
        let mut b = book();
        b.note_mumble("窗边的光很暖。", DAY_MS);
        assert_eq!(b.mumble_seq, 1);
        b.note_mumble("茶刚好入口。", DAY_MS + 5 * 60_000);
        assert_eq!(b.mumble_seq, 2);
    }

    #[test]
    fn 时长的说法() {
        assert_eq!(hours_word(45.0), "45分钟");
        assert_eq!(hours_word(120.0), "2个小时");
        assert_eq!(hours_word(150.0), "2个半小时");
        assert_eq!(hours_word(170.0), "快3个小时");
    }
}
