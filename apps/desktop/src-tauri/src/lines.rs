//! 动作台词：她开始做一件事的时候说一句。
//!
//! 「去工作了」「饿了，先吃点东西」——这些话让数值的变动有声音，而不只是面板上跳一下。
//! 台词有两个来源：`actions.toml` 里每条动作自带的默认句子（用户可以在设置里改、加），
//! 或者让本机模型按人设即兴一句（慢几秒，但每次不一样）。
//! 说出去的东西走 `pet:line` 事件：Body 出气泡，开了语音就一并念出来。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Timelike;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::chat::{self, ChatSettings};
use crate::core::actions::Catalog;
use crate::core::state_machine::{ActionRef, Mood};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LineMode {
    /// 从固定台词里随机挑一句
    Fixed,
    /// 让模型按人设即兴
    Model,
    /// 这个动作不说话
    Off,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LineSettings {
    /// 总开关：开始做事时要不要说一句。默认开
    pub enabled: bool,
    /// 全局模式，单个动作可以覆盖
    pub mode: LineMode,
    /// 两句台词之间至少隔多少秒。她换事情做的频率不高，但也别让她像复读机
    pub min_gap_sec: u32,
    /// 按动作 id 覆盖：台词列表和模式
    pub actions: HashMap<String, ActionLines>,
}

impl Default for LineSettings {
    fn default() -> Self {
        Self { enabled: true, mode: LineMode::Fixed, min_gap_sec: 45, actions: HashMap::new() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionLines {
    /// None = 跟随全局
    pub mode: Option<LineMode>,
    /// None = 用动作表里的默认台词；Some(空) = 没有固定台词（model 模式失败时就不说）
    pub lines: Option<Vec<String>>,
}

impl LineSettings {
    pub fn validated(mut self) -> Result<Self, String> {
        self.min_gap_sec = self.min_gap_sec.min(3600);
        if self.actions.len() > 64 {
            return Err("动作台词配置太多了。".into());
        }
        for (id, al) in self.actions.iter_mut() {
            if id.len() > 40 || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                return Err(format!("动作 id 无效：{id}"));
            }
            if let Some(lines) = al.lines.as_mut() {
                let cleaned: Vec<String> = lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                if cleaned.len() > 30 {
                    return Err(format!("「{id}」的台词最多 30 条。"));
                }
                if cleaned.iter().any(|l| l.chars().count() > 120) {
                    return Err(format!("「{id}」的台词每条最多 120 字。"));
                }
                *lines = cleaned;
            }
        }
        Ok(self)
    }

    fn mode_for(&self, id: &str) -> LineMode {
        self.actions.get(id).and_then(|a| a.mode).unwrap_or(self.mode)
    }

    /// 这个动作现在生效的固定台词：用户改过就用用户的，否则用动作表的
    pub fn lines_for<'a>(&'a self, cat: &'a Catalog, id: &str) -> &'a [String] {
        if let Some(Some(l)) = self.actions.get(id).map(|a| a.lines.as_ref()) {
            return l;
        }
        cat.get(id).map(|a| a.lines.as_slice()).unwrap_or(&[])
    }
}

/// 面板要看的：每个动作的 id、名字、默认台词
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInfo {
    pub id: String,
    pub name: String,
    pub default_lines: Vec<String>,
}

pub fn list_actions(cat: &Catalog) -> Vec<ActionInfo> {
    cat.all()
        .iter()
        .map(|a| ActionInfo { id: a.id.clone(), name: a.name.clone(), default_lines: a.lines.clone() })
        .collect()
}

/// Core → Body 的 `pet:line`
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub text: String,
    pub action: String,
    /// 语音要不要念。Body 还会再看一次总开关，这里只是把「台词类」标出来
    pub spoken: bool,
}

/// 上一句什么时候说的。跨线程：心跳线程和主线程都会来
#[derive(Default)]
struct LineClockState {
    last: Option<Instant>,
    by_action: HashMap<String, Instant>,
}

#[derive(Default)]
pub struct LineClock {
    state: Mutex<LineClockState>,
}

/// 自动动作台词的安静时段。提醒、对话、礼物都不走这里，仍会按用户要求出声。
/// 7:00 恰好是 sleep 的结束和早餐时段的开始，过去会在这里突然报一遍早餐台词。
fn automatic_lines_are_quiet(hour: u32) -> bool {
    hour >= 23 || hour < 8
}

fn fill(template: &str, a: &ActionRef, name: &str) -> String {
    template
        .replace("{food}", a.food.as_ref().map(|f| f.name.as_str()).unwrap_or("东西"))
        .replace("{name}", name)
        .replace("{action}", &a.name)
}

fn pick_fixed(settings: &ChatSettings, cat: &Catalog, a: &ActionRef) -> Option<String> {
    let lines = settings.lines.lines_for(cat, &a.id);
    if lines.is_empty() {
        return None;
    }
    let i = (crate::roll() * lines.len() as f32) as usize;
    Some(fill(&lines[i.min(lines.len() - 1)], a, &settings.persona.name))
}

fn emit(app: &AppHandle, a: &ActionRef, text: String, spoken: bool) {
    log::info!("{}：{}", a.name, text);
    let _ = app.emit("pet:line", Line { text, action: a.id.clone(), spoken });
}

/// 不挂在哪个动作上的一句话（生病了、没病不用吃药）。走台词总开关，不受间隔限制——
/// 这类话一天说不了几次，每一次都该让人听见
pub fn say(app: &AppHandle, tag: &str, text: &str) -> bool {
    let enabled = chat::current_settings(app).map(|s| s.lines.enabled).unwrap_or(true);
    if !enabled {
        return false;
    }
    log::info!("{tag}：{text}");
    let _ = app.emit("pet:line", Line { text: text.into(), action: tag.into(), spoken: true });
    true
}

/// 模型改写一句提醒最多等这么久；超时就说原句。「该开始了」是有时效的，不能等模型慢悠悠加载
const IN_CHARACTER_TIMEOUT: Duration = Duration::from_secs(25);

/// 把一句「事实」交给模型，用她的口吻说出来（日程提醒用）。事实里的时间、数字不能变——
/// 改写完会核对原句里每个 `HH:MM` 都还在，不在就退回原句。模型不在 / 超时 / 写砸了也退回原句。
/// 返回 false = 台词总开关关着，一个字都不会说
pub fn say_in_character(app: &AppHandle, tag: &str, facts: &str) -> bool {
    let Ok(settings) = chat::current_settings(app) else { return false };
    if !settings.lines.enabled {
        return false;
    }
    let mood = crate::pet_snapshot(app).mood;
    let app = app.clone();
    let tag = tag.to_string();
    let facts = facts.to_string();
    tauri::async_runtime::spawn(async move {
        let styled = tokio::time::timeout(IN_CHARACTER_TIMEOUT, rephrase(&settings, &facts, mood)).await;
        let text = match styled {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                log::info!("提醒没让模型改写（{e}），说原句");
                facts.clone()
            }
            Err(_) => {
                log::info!("模型改写提醒超时，说原句");
                facts.clone()
            }
        };
        log::info!("{tag}：{text}");
        let _ = app.emit("pet:line", Line { text, action: tag, spoken: true });
    });
    true
}

/// 用她的口吻复述一句事实。只给人设和事实，不给别的——4B/9B 模型给多了就开始发挥
async fn rephrase(settings: &ChatSettings, facts: &str, mood: Mood) -> Result<String, String> {
    let p = &settings.persona;
    let mood = mood_word(mood);
    let system = format!(
        "你是{}。性格：{}。说话方式：{}。你现在{}。\n\
         你要顺口提醒用户下面这件事（这是事实，时间和数字一个都不能改，也不能添加没有的安排）：\n{}\n\
         用一到两句话、不超过 40 个字，像熟悉的朋友随口提一句，可以带一点你的语气。\
         不要列清单、不要反问、不要解释。只输出这句话本身：不要引号、不要动作描写。",
        p.name, p.personality, p.speaking_style, mood, facts
    );
    let raw = chat::complete(settings, &system, "开口吧。", 80, 0.8).await?;
    let text = tidy(&raw, &p.name);
    if text.is_empty() || text.chars().count() > 70 {
        return Err(format!("不像一句提醒：{raw:?}"));
    }
    if let Some(missing) = clocks_in(facts).into_iter().find(|c| !text.contains(c.as_str())) {
        return Err(format!("把时间 {missing} 弄丢了：{text:?}"));
    }
    Ok(text)
}

/// 句子里所有 `HH:MM`。改写后核对用
fn clocks_in(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 < chars.len() {
        if chars[i].is_ascii_digit() && chars[i + 1].is_ascii_digit() && chars[i + 2] == ':' && chars[i + 3].is_ascii_digit() && chars[i + 4].is_ascii_digit() {
            out.push(chars[i..i + 5].iter().collect());
            i += 5;
        } else {
            i += 1;
        }
    }
    out
}

/// 她刚开始做 `a`。`force` = 不受间隔限制（收礼物必须当场有反应）。
/// 固定台词立刻发；模型即兴放到后台，几秒后再发
pub fn announce(app: &AppHandle, a: &ActionRef, force: bool) {
    let settings = match chat::current_settings(app) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("读不到设置，这句台词不说了：{e}");
            return;
        }
    };
    if !settings.lines.enabled {
        return;
    }
    let mode = settings.lines.mode_for(&a.id);
    if mode == LineMode::Off {
        return;
    }
    if !force && automatic_lines_are_quiet(chrono::Local::now().hour()) {
        log::info!("安静时段跳过自动动作台词：{}", a.name);
        return;
    }
    if let Some(clock) = app.try_state::<LineClock>() {
        let Ok(mut state) = clock.state.lock() else { return };
        let gap = Duration::from_secs(settings.lines.min_gap_sec as u64);
        if !force {
            if state.last.is_some_and(|t| t.elapsed() < gap) {
                return;
            }
            // 同一个动作短时间来回切换时不要复读。十分钟只约束自动台词；
            // 礼物等明确交互用 force=true，仍然每次都有回应。
            let same_action_gap = Duration::from_secs(settings.lines.min_gap_sec.max(600) as u64);
            if state.by_action.get(&a.id).is_some_and(|t| t.elapsed() < same_action_gap) {
                return;
            }
        }
        let now = Instant::now();
        state.last = Some(now);
        state.by_action.insert(a.id.clone(), now);
        state.by_action.retain(|_, at| at.elapsed() < Duration::from_secs(7200));
    }
    let cat = app.state::<Catalog>();
    let spoken = settings.voice.enabled && settings.voice.speak_lines;
    match mode {
        LineMode::Fixed | LineMode::Off => {
            if let Some(text) = pick_fixed(&settings, &cat, a) {
                emit(app, a, text, spoken);
            }
        }
        LineMode::Model => {
            let fallback = pick_fixed(&settings, &cat, a);
            let app = app.clone();
            let a = a.clone();
            let mood = crate::pet_snapshot(&app).mood;
            tauri::async_runtime::spawn(async move {
                match improvise(&settings, &a, mood).await {
                    Ok(text) => emit(&app, &a, text, spoken),
                    Err(e) => {
                        log::warn!("模型没写出台词（{e}），用固定的");
                        if let Some(text) = fallback {
                            emit(&app, &a, text, spoken);
                        }
                    }
                }
            });
        }
    }
}

fn mood_word(m: Mood) -> &'static str {
    match m {
        Mood::Happy => "心情很好",
        Mood::Nomal => "心情一般",
        Mood::PoorCondition => "状态不太好，有点累",
        Mood::Ill => "生病了，没什么精神",
    }
}

/// 让模型即兴一句。系统提示只给人设和处境，预填充「名字：」让它直接开口
async fn improvise(settings: &ChatSettings, a: &ActionRef, mood: Mood) -> Result<String, String> {
    let p = &settings.persona;
    let what = match a.food.as_ref() {
        Some(f) => format!("{}（{}）", a.name, f.name),
        None => a.name.clone(),
    };
    let system = format!(
        "你是{}。性格：{}。说话方式：{}。\n你现在{}，刚开始「{}」，原因是：{}。\n\
         用一句话（不超过 25 个字）说出你此刻脱口而出的话，像自言自语或对身边的人随口一说。\
         只输出这一句话本身：不要引号、不要解释、不要动作描写。",
        p.name, p.personality, p.speaking_style, mood_word(mood), what, a.reason
    );
    let raw = chat::complete(settings, &system, "开口吧。", 48, 0.9).await?;
    let text = tidy(&raw, &p.name);
    if text.is_empty() || text.chars().count() > 60 {
        return Err(format!("不像一句台词：{raw:?}"));
    }
    Ok(text)
}

/// 面板的「让模型写几条」：一次给 n 句，用户再挑着留
pub async fn draft_lines(settings: &ChatSettings, cat: &Catalog, id: &str, n: usize) -> Result<Vec<String>, String> {
    let a = cat.get(id).ok_or_else(|| format!("没有这个动作：{id}"))?;
    let p = &settings.persona;
    let n = n.clamp(1, 10);
    let hint = if a.has_tag("eat") || a.has_tag("drink") || a.has_tag("gift") {
        "可以用 {food} 代表她手里的东西。"
    } else {
        ""
    };
    let system = format!(
        "你是{}。性格：{}。说话方式：{}。\n请写 {} 句她开始「{}」时可能随口说的话，每句不超过 25 个字，\
         彼此语气不同。{}每行一句，不要编号、不要引号、不要解释。",
        p.name, p.personality, p.speaking_style, n, a.name, hint
    );
    let raw = chat::complete(settings, &system, "开始写。", 64 * n as u32, 0.95).await?;
    let lines: Vec<String> = raw
        .lines()
        .map(|l| tidy(l, &p.name))
        .filter(|l| !l.is_empty() && l.chars().count() <= 60)
        .take(n)
        .collect();
    if lines.is_empty() {
        return Err("模型没写出像样的台词，再试一次。".into());
    }
    Ok(lines)
}

/// 去掉编号、引号、名字前缀这些模型爱加的东西
fn tidy(line: &str, name: &str) -> String {
    let mut s = line.trim();
    // 「1. 」「1、」「- 」
    s = s.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | '、' | ')' | '）' | '-' | '•' | ' '));
    // 每一行都可能被它冠上「萝莉斯：」
    for prefix in [format!("{name}："), format!("{name}:")] {
        if let Some(rest) = s.strip_prefix(prefix.as_str()) {
            s = rest.trim_start();
        }
    }
    s.trim_matches(|c: char| matches!(c, '"' | '“' | '”' | '「' | '」' | '\'' | ' ')).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state_machine::FoodRef;

    #[test]
    fn 改写提醒时时间一个都不能丢() {
        assert_eq!(clocks_in("今天排了 3 件事，20:00 从「高数」开始；21:30 第二件"), ["20:00", "21:30"]);
        assert!(clocks_in("有 3 篇笔记到期该复习了").is_empty());
    }

    fn action(id: &str, food: Option<&str>) -> ActionRef {
        ActionRef {
            id: id.into(),
            name: id.into(),
            graph: id.into(),
            reason: "测试".into(),
            food: food.map(|f| FoodRef { id: f.into(), name: f.into() }),
        }
    }

    #[test]
    fn 动作表的默认台词能填模板() {
        let cat = Catalog::load();
        let settings = ChatSettings::default();
        let a = action("meal", Some("包子"));
        let lines = settings.lines.lines_for(&cat, "meal");
        assert!(!lines.is_empty(), "吃饭该有默认台词");
        let text = fill(&lines[0], &a, "萝莉斯");
        assert!(!text.contains("{food}"), "{text}");
    }

    #[test]
    fn 用户改过的台词优先() {
        let cat = Catalog::load();
        let mut settings = ChatSettings::default();
        settings.lines.actions.insert("meal".into(), ActionLines { mode: None, lines: Some(vec!["开饭！".into()]) });
        assert_eq!(settings.lines.lines_for(&cat, "meal"), &["开饭！".to_string()]);
        // 显式清空 = 没台词
        settings.lines.actions.insert("meal".into(), ActionLines { mode: None, lines: Some(vec![]) });
        assert!(settings.lines.lines_for(&cat, "meal").is_empty());
        assert!(pick_fixed(&settings, &cat, &action("meal", None)).is_none());
    }

    #[test]
    fn 单个动作可以覆盖模式() {
        let mut s = LineSettings::default();
        assert_eq!(s.mode_for("rest"), LineMode::Fixed);
        s.actions.insert("rest".into(), ActionLines { mode: Some(LineMode::Off), lines: None });
        assert_eq!(s.mode_for("rest"), LineMode::Off);
        assert_eq!(s.mode_for("meal"), LineMode::Fixed);
    }

    #[test]
    fn 校验会清理空行并拒绝过长() {
        let mut s = LineSettings::default();
        s.actions.insert("meal".into(), ActionLines { mode: None, lines: Some(vec![" 吃饭 ".into(), "".into()]) });
        let ok = s.clone().validated().unwrap();
        assert_eq!(ok.actions["meal"].lines.as_ref().unwrap(), &["吃饭".to_string()]);
        s.actions.insert("meal".into(), ActionLines { mode: None, lines: Some(vec!["字".repeat(200)]) });
        assert!(s.clone().validated().is_err());
        s.actions.clear();
        s.actions.insert("../x".into(), ActionLines::default());
        assert!(s.validated().is_err());
    }

    #[test]
    fn 整理模型输出() {
        assert_eq!(tidy("1. 「去工作啦」", "萝莉斯"), "去工作啦");
        assert_eq!(tidy("- \"好累啊\"", "萝莉斯"), "好累啊");
        assert_eq!(tidy("  开饭咯～  ", "萝莉斯"), "开饭咯～");
        assert_eq!(tidy("2、", "萝莉斯"), "");
        assert_eq!(tidy("3. 萝莉斯：哎呀，你输了！", "萝莉斯"), "哎呀，你输了！");
    }

    #[test]
    fn 自动台词在夜间和早上七点静音() {
        for hour in [23, 0, 3, 7] {
            assert!(automatic_lines_are_quiet(hour), "{hour} 点应该静音");
        }
        for hour in [8, 12, 22] {
            assert!(!automatic_lines_are_quiet(hour), "{hour} 点应该允许动作台词");
        }
    }
}
