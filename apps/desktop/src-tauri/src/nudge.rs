//! 主动提醒（Nudge Policy）：按知识库的日程，短短一句，不吵。
//!
//! 全是确定性规则，没有模型参与——「什么时候开口」不该靠概率。三道刹车：
//!
//! 1. **只说一件事**，一句话。早上一条今日概览，每件事开始前一条「该开始了」。
//! 2. **说过就不再说**：每条提醒有一个 key，按天记在 `kv` 里，重启也不重复。
//!    两条之间至少隔 `MIN_GAP_MIN`。
//! 3. **人不在就闭嘴**：调用方（lib.rs）只在电脑清醒、你最近有过键鼠操作、
//!    不在夜里、她自己也没睡着的时候才来问 `pick`。刚从待机醒来的几分钟也不说——
//!    那会儿你在忙别的。
//!
//! `pick` 是纯函数：现在几点、今天的安排、已经说过哪些，进；要不要说、说什么，出。
//! 把「今天的安排」写进她的记忆是调用方的事（`remember` 里的 `system_event`）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::kb::{Agenda, KbEvent};

/// 开始前多少分钟提醒
pub const DEFAULT_LEAD_MIN: u32 = 15;
/// 两条提醒至少隔多久（分钟）。「该开始了」是有时效的，可以近一点
pub const MIN_GAP_MIN: f32 = 30.0;
pub const MIN_GAP_SOON_MIN: f32 = 5.0;
/// 今日概览最早几点说。和作息 / 台词的静音时段一致：8 点她才起
pub const BRIEF_FROM_HOUR: u32 = 8;
/// 今日概览最晚几点还值得说：中午以后才开机就直接等「该开始了」
pub const BRIEF_UNTIL_HOUR: u32 = 14;
/// 你多久没动键鼠就算不在（秒）
pub const DEFAULT_IDLE_MAX_SEC: u32 = 300;
/// 从待机醒来后先安静这么久（分钟）
pub const WAKE_GRACE_MIN: f32 = 3.0;
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
}

impl Default for NudgeSettings {
    fn default() -> Self {
        Self { enabled: true, lead_min: DEFAULT_LEAD_MIN, idle_max_sec: DEFAULT_IDLE_MAX_SEC }
    }
}

/// 一条要说的话。`key` 记到「说过了」里；`remember` 是顺手写进她记忆的一句（只有概览有）
#[derive(Debug, Clone, PartialEq)]
pub struct Nudge {
    pub key: String,
    pub text: String,
    pub remember: Option<String>,
}

/// 现在该不该开口。`minute_of_day` 从零点起的分钟；`since_last_min` 上一条说了多久
/// （None = 今天还没说过）
pub fn pick(
    agenda: &Agenda,
    minute_of_day: u32,
    said: &HashSet<String>,
    since_last_min: Option<f32>,
    lead_min: u32,
) -> Option<Nudge> {
    let gap_ok = |need: f32| since_last_min.map_or(true, |m| m >= need);

    // 1. 马上要开始的事：时效最高，排前面
    if gap_ok(MIN_GAP_SOON_MIN) {
        for e in agenda.pending() {
            let Some(start) = e.start else { continue };
            let key = format!("soon:{}", e.id);
            // 开始前 lead 分钟到开始后 1 分钟这个窗口里说一次；过了窗口就不追着说了
            let in_window = start <= minute_of_day + lead_min && minute_of_day <= start + 1;
            if in_window && !said.contains(&key) {
                return Some(Nudge { key, text: format!("{} 该开始「{}」了。", clock(start), short(&e.title)), remember: None });
            }
        }
    }

    // 2. 今日概览：早上第一次看到你的时候
    let hour = minute_of_day / 60;
    let key = format!("brief:{}", agenda.date);
    if (BRIEF_FROM_HOUR..BRIEF_UNTIL_HOUR).contains(&hour) && !said.contains(&key) && gap_ok(MIN_GAP_MIN) {
        return brief(agenda).map(|(text, remember)| Nudge { key, text, remember: Some(remember) });
    }
    None
}

/// 你主动问「今天有什么安排」时用：不看钟点、不看说没说过，直接给概览
pub fn brief_now(agenda: &Agenda) -> Option<Nudge> {
    brief(agenda).map(|(text, remember)| Nudge { key: format!("brief:{}", agenda.date), text, remember: Some(remember) })
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

    #[test]
    fn 早上一条概览_简短_并写记忆() {
        let n = pick(&day(), 9 * 60, &HashSet::new(), None, 15).expect("早上该有概览");
        assert_eq!(n.key, "brief:2026-09-18");
        assert_eq!(n.text, "今天排了 3 件事，20:00 从「高数 p43 42 有理函数的积分（上集）」开始；有 3 篇笔记到期该复习了。");
        assert!(n.text.chars().count() < 60, "太长了：{}", n.text);
        let mem = n.remember.unwrap();
        assert!(mem.starts_with("9月18日 的安排：20:00–21:00 高数 p43"), "{mem}");
        assert!(mem.contains("3 篇笔记"));
    }

    #[test]
    fn 概览只说一次_夜里和下午不说() {
        let said: HashSet<String> = ["brief:2026-09-18".to_string()].into();
        assert_eq!(pick(&day(), 9 * 60, &said, None, 15), None, "说过了");
        assert_eq!(pick(&day(), 3 * 60, &HashSet::new(), None, 15), None, "凌晨");
        assert_eq!(pick(&day(), 15 * 60, &HashSet::new(), None, 15), None, "下午才开机就等「该开始了」");
    }

    #[test]
    fn 今天没事就闭嘴() {
        let a = Agenda { date: "2026-09-18".into(), events: vec![ev("x", "做完的", Some(600), true)], reviews_due: 0 };
        assert_eq!(pick(&a, 9 * 60, &HashSet::new(), None, 15), None);
    }

    #[test]
    fn 开始前十五分钟提醒一次() {
        let said = HashSet::new();
        assert_eq!(pick(&day(), 19 * 60 + 30, &said, Some(60.0), 15), None, "还早");
        let n = pick(&day(), 19 * 60 + 46, &said, Some(60.0), 15).unwrap();
        assert_eq!(n.key, "soon:a");
        assert_eq!(n.text, "20:00 该开始「高数 p43 42 有理函数的积分（上集）」了。");
        assert!(n.remember.is_none());
        // 说过了就不重复；过了开始一分钟也不追着说
        let said: HashSet<String> = ["soon:a".to_string()].into();
        assert_eq!(pick(&day(), 19 * 60 + 50, &said, Some(4.0), 15), None);
        assert_eq!(pick(&day(), 20 * 60 + 10, &HashSet::new(), Some(60.0), 15), None, "已经开始十分钟了，不追");
    }

    #[test]
    fn 两条之间要隔一会儿_但时效的可以近一点() {
        let said: HashSet<String> = ["brief:2026-09-18".to_string()].into();
        assert_eq!(pick(&day(), 19 * 60 + 50, &said, Some(2.0), 15), None, "刚说过两分钟");
        assert!(pick(&day(), 19 * 60 + 50, &said, Some(6.0), 15).is_some(), "六分钟够了");
        assert_eq!(pick(&day(), 9 * 60, &HashSet::new(), Some(10.0), 15), None, "概览要隔半小时");
    }

    #[test]
    fn 做完的不提醒_标题太长会截() {
        let mut a = day();
        a.events[0].done = true;
        let n = pick(&a, 20 * 60 + 50, &HashSet::new(), None, 15).unwrap();
        assert_eq!(n.key, "soon:b");
        let long = ev("l", &"很长".repeat(30), Some(600), false);
        assert!(short(&long.title).chars().count() == TITLE_MAX_CHARS);
    }
}
