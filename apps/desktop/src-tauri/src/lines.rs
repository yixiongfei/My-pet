//! 动作台词：她开始做一件事的时候说一句。
//!
//! 「去工作了」「饿了，先吃点东西」——这些话让数值的变动有声音，而不只是面板上跳一下。
//! 台词有两个来源：`actions.toml` 里每条动作自带的默认句子（用户可以在设置里改、加），
//! 或者让本机模型按人设即兴一句（慢几秒，但每次不一样）。
//! 说出去的东西走 `pet:line` 事件：Body 出气泡，开了语音就一并念出来。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
pub struct LineClock {
    last: Mutex<Option<Instant>>,
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
    if let Some(clock) = app.try_state::<LineClock>() {
        let Ok(mut last) = clock.last.lock() else { return };
        let gap = Duration::from_secs(settings.lines.min_gap_sec as u64);
        if !force && last.is_some_and(|t| t.elapsed() < gap) {
            return;
        }
        *last = Some(Instant::now());
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
}
