//! 对话里的「使唤」（roadmap 2.8 的自然语言入口）。
//!
//! 「去玩会儿」「休息一下吧」「go take a nap」这类话不该只换来一句闲聊——
//! 它们要进状态机过一次服从判定，答应了就真的去做。这里只做**识别**，
//! 判定和执行还是 `obey::judge` / `state_machine::request` 的事：用户说什么
//! 不能直接写 `state.activity`，这条边界在这里也不松。
//!
//! 全部是规则，不走模型：命令得在几毫秒内认出来，而且必须可预测——
//! 「你昨天玩了什么」被当成「去玩」，比漏掉一句「去玩吧」糟糕得多，所以
//! 规则偏保守：拿不准的一律当闲聊。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Intent {
    /// 一次性请求：去做某件事。`target` 是动作 id 或 tag，`minutes` 是用户说的时长
    Do { target: String, minutes: Option<f32> },
    /// 持续倾向：多做（正）/ 少做（负），走 `Event::SetBias`
    Bias { tag: String, weight: f32 },
    /// 专注一段时间：「设个番茄钟，学习一个小时」——头顶出倒计时，期间她去做 `target`
    Focus { minutes: f32, target: Option<String> },
    /// 单纯的提醒：「十分钟后叫我」
    Timer { minutes: f32, label: String },
}

/// 没说多久的番茄钟
pub const DEFAULT_FOCUS_MIN: f32 = 25.0;

/// 中文动词 → 目标。**顺序有讲究**：长的在前（「玩游戏」先于「玩」），
/// 否则「打游戏」会被「打工」抢走
const ZH_VERBS: &[(&str, &str)] = &[
    ("玩游戏", "play"), ("打游戏", "play"), ("玩会儿", "play"), ("玩一会", "play"), ("放松", "play"),
    ("玩水", "play"), ("跳绳", "play"), ("玩", "play"),
    ("休息", "rest"), ("歇会儿", "rest"), ("歇一会", "rest"), ("歇一歇", "rest"), ("歇歇", "rest"),
    ("歇着", "rest"), ("歇", "rest"), ("发呆", "rest"), ("缓一缓", "rest"), ("缓缓", "rest"),
    ("午睡", "sleep"), ("睡觉", "sleep"), ("睡一会", "sleep"), ("睡会儿", "sleep"), ("睡吧", "sleep"), ("睡", "sleep"),
    ("工作", "work"), ("干活", "work"), ("上班", "work"), ("打工", "work"), ("挣钱", "work"), ("赚钱", "work"), ("搬砖", "work"),
    ("学习", "study"), ("看书", "study"), ("读书", "study"), ("念书", "study"), ("写作业", "study"), ("做作业", "study"),
    ("练字", "study"), ("画画", "study"), ("学点东西", "study"), ("学点儿", "study"), ("学一会", "study"), ("学会儿", "study"),
    ("吃饭", "eat"), ("吃点东西", "eat"), ("吃东西", "eat"), ("吃个饭", "eat"), ("吃口饭", "eat"), ("吃点儿", "eat"),
    ("吃点", "eat"), ("加餐", "eat"), ("吃零食", "eat"), ("吃", "eat"),
    ("喝水", "drink"), ("喝点水", "drink"), ("喝口水", "drink"), ("喝点东西", "drink"), ("喝", "drink"),
];

/// 明确是对她说的祈使标记。含其一即视为命令（再过一遍排除规则）
const ZH_ADDRESSED: &[&str] = &[
    "让你", "叫你", "要你", "请你", "你去", "你先", "你快", "你该", "你也", "你就", "你得", "你现在",
    "你可以", "你应该", "你要", "你来", "你该去", "你去", "带你",
];
/// 句首允许的前缀（去掉之后动词得顶头）。前面几个是招呼语：「晚安，睡觉去吧」
const ZH_LEADING: &[&str] = &[
    "晚安", "早安", "好了", "好啦", "好的", "那就", "那你", "那", "嗯", "行", "好", "你",
    "快去", "先去", "现在去", "去", "快", "先", "该", "请", "麻烦", "帮我", "来", "现在", "一起", "也",
];
/// 在问她，不是在使唤她：「你去哪儿玩了」「你喜欢玩什么」
const ZH_QUESTION: &[&str] = &["哪", "什么", "怎么", "为什么", "谁", "是不是", "有没有", "会不会", "喜欢", "多久", "几点", "几个", "多少"];
/// 句尾的商量口气，也算命令
const ZH_TRAILING: &[&str] = &["吧", "呗", "好不好", "好吗", "行吗", "可以吗", "行不行", "怎么样", "如何", "去"];

/// 动词后面紧跟这些就是在说过去 / 问结果，不是命令
const ZH_PAST: &[&str] = &["了吗", "过", "没", "了没"];

const EN_VERBS: &[(&str, &str)] = &[
    ("play a game", "play"), ("play games", "play"), ("play", "play"), ("have fun", "play"), ("relax", "play"),
    ("take a break", "rest"), ("take a rest", "rest"), ("have a rest", "rest"), ("rest", "rest"), ("chill", "rest"),
    ("take a nap", "sleep"), ("nap", "sleep"), ("sleep", "sleep"), ("go to bed", "sleep"),
    ("work", "work"), ("study", "study"), ("read", "study"), ("learn", "study"),
    ("have a meal", "eat"), ("grab a bite", "eat"), ("eat", "eat"),
    ("drink some water", "drink"), ("drink water", "drink"), ("drink", "drink"),
];
const EN_LEADING: &[&str] = &[
    "please", "now", "go", "go and", "go to", "time to", "it's time to", "its time to", "let's", "lets", "why don't you",
    "why not", "you should", "you can", "you may", "you need to", "you have to", "you'd better", "could you", "can you",
    "would you", "i want you to", "i'd like you to", "i need you to", "maybe",
];

/// 认一句话。认不出来就是 None：交给正常对话
pub fn parse(text: &str) -> Option<Intent> {
    let t = normalize(text);
    if t.is_empty() || t.chars().count() > 60 {
        return None; // 长篇大论不会是一句命令
    }
    parse_focus(&t)
        .or_else(|| parse_timer(&t))
        .or_else(|| parse_bias_zh(&t))
        .or_else(|| parse_do_zh(&t))
        .or_else(|| parse_bias_en(&t))
        .or_else(|| parse_do_en(&t))
}

/// 「帮我设个番茄钟，学习一个小时吧」「专注 50 分钟」「start a 30 min pomodoro」
fn parse_focus(t: &str) -> Option<Intent> {
    let s = strip_punct(t);
    if !["番茄钟", "番茄", "专注", "pomodoro", "focus session", "focus for", "focus mode", "deep work"].iter().any(|k| s.contains(k)) {
        return None;
    }
    if s.contains("取消") || s.contains("停") || s.contains("cancel") || s.contains("stop") {
        return None; // 「停掉番茄钟」交给别的地方，这里只管开
    }
    let (minutes, _) = duration_zh(&s);
    let minutes = minutes.or_else(|| duration_en(t).0).unwrap_or(DEFAULT_FOCUS_MIN);
    // 期间让她干什么：句子里提到学习 / 工作就跟着做，没提就只是计时
    let target = find_verb_zh(&s)
        .map(|(_, _, tag)| tag)
        .filter(|tag| ["study", "work", "play", "rest"].contains(tag))
        .or_else(|| {
            if t.contains("study") || t.contains("read") { Some("study") }
            else if t.contains("work") { Some("work") }
            else { None }
        })
        .map(String::from);
    Some(Intent::Focus { minutes: minutes.clamp(1.0, 240.0), target })
}

/// 「十分钟后叫我」「提醒我 20 分钟后喝水」「remind me in 15 minutes to stretch」
fn parse_timer(t: &str) -> Option<Intent> {
    let s = strip_punct(t);
    let zh_hit = ["提醒我", "叫我", "闹钟", "计时", "倒计时", "定个时"].iter().any(|k| s.contains(k));
    let en_hit = t.contains("remind me") || t.contains("timer") || t.contains("alarm") || t.contains("wake me");
    if !zh_hit && !en_hit {
        return None;
    }
    let (minutes, rest) = duration_zh(&s);
    let (minutes, rest) = match minutes {
        Some(m) => (m, rest),
        None => {
            let (m, rest_en) = duration_en(t);
            (m?, rest_en)
        }
    };
    // 标签：去掉「提醒我」「后」「叫我」这些骨架，剩下的就是要提醒的事
    let mut label = rest;
    for k in ["提醒我", "叫我一下", "叫我", "闹钟", "定个时", "计时", "倒计时", "帮我", "请", "记得", "后", "之后", "到了", "remind me", "in ", "to ", "set a", "set ", "timer", "alarm", "for", "please", "wake me up", "wake me"] {
        label = label.replace(k, " ");
    }
    let label = label.trim_matches(|c: char| c.is_whitespace() || matches!(c, '，' | ',' | '吧' | '哦' | '啊' | '呀' | '~')).trim().to_string();
    Some(Intent::Timer { minutes: minutes.clamp(0.5, 24.0 * 60.0), label: if label.is_empty() { "提醒".into() } else { label } })
}

/// 全角标点 → 半角，去掉空白（中文不靠空格分词），统一小写
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.trim().chars() {
        out.push(match c {
            '，' => ',', '。' => '.', '！' => '!', '？' => '?', '：' => ':', '；' => ';', '～' => '~',
            '（' => '(', '）' => ')', '　' => ' ',
            other => other,
        });
    }
    out.to_lowercase()
}

fn strip_punct(t: &str) -> String {
    t.chars().filter(|c| !matches!(c, '.' | '!' | '?' | ',' | '~' | '…' | ' ' | '\n' | '\t')).collect()
}

fn find_verb_zh(t: &str) -> Option<(usize, &'static str, &'static str)> {
    ZH_VERBS
        .iter()
        .filter_map(|(v, tag)| t.find(v).map(|i| (i, *v, *tag)))
        .min_by_key(|(i, v, _)| (*i, std::cmp::Reverse(v.len())))
}

/// 「多工作一点」「少玩点」「别玩了」——持续倾向，只认 work / study / play
fn parse_bias_zh(t: &str) -> Option<Intent> {
    let s = strip_punct(t);
    if s.contains("我") && !ZH_ADDRESSED.iter().any(|m| s.contains(m)) {
        return None;
    }
    let (i, verb, tag) = find_verb_zh(&s)?;
    if !["work", "study", "play"].contains(&tag) {
        return None;
    }
    let before = &s[..i];
    let after = &s[i + verb.len()..];
    let more = before.ends_with('多') || after.starts_with("多一点") || after.starts_with("多点") || after.starts_with("多些") || after.starts_with("多一些");
    let less = before.ends_with('少') || after.starts_with("少一点") || after.starts_with("少点") || after.starts_with("少些")
        || ((before.ends_with("别") || before.ends_with("不要") || before.ends_with("不许") || before.ends_with("不准") || before.ends_with("不用"))
            && (after.starts_with('了') || after.is_empty() || after.starts_with("啦")));
    if more {
        Some(Intent::Bias { tag: tag.into(), weight: 1.0 })
    } else if less {
        Some(Intent::Bias { tag: tag.into(), weight: -1.0 })
    } else {
        None
    }
}

fn parse_do_zh(t: &str) -> Option<Intent> {
    let s = strip_punct(t);
    if s.is_empty() {
        return None;
    }
    let (i, verb, tag) = find_verb_zh(&s)?;
    let after = &s[i + verb.len()..];
    if ZH_PAST.iter().any(|p| after.starts_with(p)) || (after.starts_with('了') && after != "了") {
        return None; // 「吃过了」「玩了吗」「玩了一天」是在聊，不是在使唤
    }
    if ZH_QUESTION.iter().any(|q| s.contains(q)) {
        return None;
    }
    // 「一起去玩吧」也算对她说的
    let addressed = ZH_ADDRESSED.iter().any(|m| s.contains(m)) || s.contains("一起");
    // 说的是「我」而不是「你」——「我去吃饭了」「我想休息」
    if !addressed && (s.contains('我') || s.contains("咱")) {
        return None;
    }
    // 否定词：「别玩了」走的是 bias，这里不当命令
    if s.contains("别") || s.contains("不要") || s.contains("不许") || s.contains("不准") || s.contains("不用") || s.contains("不能") {
        return None;
    }
    // 剥掉句首的招呼语和祈使前缀（最多三层：「好了，你快去睡觉」）
    let mut head = &s[..i];
    for _ in 0..3 {
        let Some(rest) = ZH_LEADING.iter().find_map(|p| head.strip_prefix(p)) else { break };
        head = rest;
    }
    let (minutes, after_no_dur) = duration_zh(after);
    // 「该睡觉了」「快去吃饭了」是命令；光秃秃的「吃饭了」可能只是在说自己
    let urged = s.contains('该') || s.contains('快') || s.contains("到点") || s.contains("时间");
    let trailing = ZH_TRAILING.iter().any(|p| after_no_dur.ends_with(p))
        || after_no_dur.is_empty()
        || (after_no_dur == "了" && urged)
        || after_no_dur == "会儿" || after_no_dur == "一会儿" || after_no_dur == "一会" || after_no_dur == "一下"
        || after_no_dur.ends_with("一会儿吧") || after_no_dur.ends_with("一下吧") || after_no_dur.ends_with("会儿吧");
    let leading_ok = head.is_empty();
    if !(addressed || (leading_ok && (trailing || minutes.is_some()))) {
        return None;
    }
    if !trailing && minutes.is_none() && after_no_dur.chars().count() > 12 {
        return None; // 后面还有一大截，多半是在讲事情，不是一句命令
    }
    Some(Intent::Do { target: tag.into(), minutes })
}

/// 从「玩十分钟吧」里抠出 10，返回 (分钟, 去掉时长后的剩余文本)
pub(crate) fn duration_zh(after: &str) -> (Option<f32>, String) {
    let mut rest = after.to_string();
    let mut minutes = None;
    // 数字 + 单位
    let chars: Vec<char> = rest.chars().collect();
    for (i, _) in chars.iter().enumerate() {
        let (n, used) = read_number_zh(&chars[i..]);
        if used == 0 {
            continue;
        }
        let tail: String = chars[i + used..].iter().collect();
        // 「一个半小时」的「半」夹在量词和单位中间，所以按整串匹配
        for (unit, mult, extra) in [
            ("个半小时", 60.0, 30.0), ("个半钟头", 60.0, 30.0), ("个小时", 60.0, 0.0), ("个钟头", 60.0, 0.0),
            ("小时半", 60.0, 30.0), ("小时", 60.0, 0.0), ("钟头", 60.0, 0.0), ("分钟", 1.0, 0.0), ("分", 1.0, 0.0),
        ] {
            if let Some(after_unit) = tail.strip_prefix(unit) {
                let m = n * mult + extra;
                if m > 0.0 {
                    minutes = Some(m);
                    let head: String = chars[..i].iter().collect();
                    return (minutes, head + after_unit);
                }
            }
        }
    }
    for (phrase, m) in [("半个小时", 30.0), ("半小时", 30.0), ("一整天", 240.0), ("一天", 240.0), ("一上午", 180.0), ("一下午", 180.0), ("一晚上", 240.0), ("整晚", 240.0)] {
        if let Some(i) = rest.find(phrase) {
            minutes = Some(m);
            rest.replace_range(i..i + phrase.len(), "");
            return (minutes, rest);
        }
    }
    (minutes, rest)
}

/// 读一个中文或阿拉伯数字，返回 (值, 用掉几个字符)
fn read_number_zh(chars: &[char]) -> (f32, usize) {
    let mut i = 0;
    let mut digits = String::new();
    while i < chars.len() && chars[i].is_ascii_digit() {
        digits.push(chars[i]);
        i += 1;
    }
    if !digits.is_empty() {
        return (digits.parse().unwrap_or(0.0), i);
    }
    let value = |c: char| match c {
        '零' => Some(0.0), '一' => Some(1.0), '两' | '二' => Some(2.0), '三' => Some(3.0), '四' => Some(4.0),
        '五' => Some(5.0), '六' => Some(6.0), '七' => Some(7.0), '八' => Some(8.0), '九' => Some(9.0),
        _ => None,
    };
    let mut total = 0.0;
    let mut cur = 0.0;
    let mut used = 0;
    while used < chars.len() {
        let c = chars[used];
        if c == '十' {
            total += if cur == 0.0 { 10.0 } else { cur * 10.0 };
            cur = 0.0;
            used += 1;
        } else if let Some(v) = value(c) {
            cur = v;
            used += 1;
        } else {
            break;
        }
    }
    (total + cur, used)
}

fn parse_bias_en(t: &str) -> Option<Intent> {
    let s = t.trim_matches(|c: char| !c.is_alphanumeric());
    let tags = [("work", "work"), ("study", "study"), ("play", "play"), ("game", "play"), ("games", "play")];
    for (word, tag) in tags {
        let more = s.contains(&format!("{word} more")) || s.contains(&format!("more {word}"));
        let less = s.contains(&format!("{word} less")) || s.contains(&format!("less {word}")) || s.contains(&format!("stop {word}"))
            || s.contains(&format!("stop playing")) && tag == "play"
            || s.contains(&format!("don't {word}")) || s.contains(&format!("do not {word}")) || s.contains(&format!("no more {word}"));
        if more {
            return Some(Intent::Bias { tag: tag.into(), weight: 1.0 });
        }
        if less {
            return Some(Intent::Bias { tag: tag.into(), weight: -1.0 });
        }
    }
    None
}

fn parse_do_en(t: &str) -> Option<Intent> {
    // 只处理拉丁文本；带中文的已经在上面认过了
    if !t.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut s: String = t.chars().filter(|c| !matches!(c, '.' | '!' | '?' | ',' | '~')).collect::<String>().trim().to_string();
    if s.starts_with("i ") || s.starts_with("i'") || s.starts_with("we ") || s.starts_with("im ") || s.starts_with("i am") {
        if !s.contains("you") {
            return None; // "I want to play" 说的是自己
        }
    }
    if s.contains("don't") || s.contains("do not") || s.contains("stop ") || s.contains("never") {
        return None;
    }
    let (minutes, stripped) = duration_en(&s);
    s = stripped;
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    // 动词得在句首（去掉引导语之后），或者句子明确对「you」说
    let mut body = s.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for p in EN_LEADING {
            if let Some(rest) = body.strip_prefix(p) {
                if rest.is_empty() || rest.starts_with(' ') {
                    body = rest.trim_start().to_string();
                    changed = true;
                }
            }
        }
        for p in ["you ", "then ", "and ", "just ", "to "] {
            if let Some(rest) = body.strip_prefix(p) {
                body = rest.trim_start().to_string();
                changed = true;
            }
        }
    }
    let addressed = s.contains("you");
    for (verb, tag) in EN_VERBS {
        let hit = body == *verb
            || body.starts_with(&format!("{verb} "))
            || (addressed && (s.contains(&format!(" {verb} ")) || s.ends_with(&format!(" {verb}"))));
        if !hit {
            continue;
        }
        // 问过去的事："did you play" / "have you eaten"
        if s.starts_with("did ") || s.starts_with("have you") || s.starts_with("what ") || s.starts_with("when ") || s.starts_with("where ") || s.starts_with("how ") || s.starts_with("are you") || s.starts_with("do you") {
            return None;
        }
        return Some(Intent::Do { target: (*tag).into(), minutes });
    }
    None
}

fn duration_en(s: &str) -> (Option<f32>, String) {
    let mut out = s.to_string();
    for (phrase, m) in [("half an hour", 30.0), ("half hour", 30.0), ("an hour", 60.0), ("one hour", 60.0), ("a minute", 1.0)] {
        if let Some(i) = out.find(phrase) {
            out.replace_range(i..i + phrase.len(), "");
            return (Some(m), out.trim().to_string());
        }
    }
    let words: Vec<String> = out.split_whitespace().map(String::from).collect();
    for (i, w) in words.iter().enumerate() {
        let Ok(n) = w.parse::<f32>() else { continue };
        let Some(unit) = words.get(i + 1) else { continue };
        let mult = match unit.as_str() {
            "min" | "mins" | "minute" | "minutes" | "m" => 1.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 60.0,
            _ => continue,
        };
        let mut rest = words.clone();
        rest.drain(i..=i + 1);
        return (Some(n * mult), rest.join(" "));
    }
    (None, out)
}

/// 给模型看的说明：她刚才答应 / 拒绝了什么
pub fn describe_target(target: &str) -> &'static str {
    match target {
        "play" => "去玩",
        "rest" => "休息",
        "sleep" => "睡觉",
        "work" => "去工作",
        "study" => "去学习",
        "eat" => "去吃饭",
        "drink" => "去喝水",
        _ => "做那件事",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go(text: &str) -> Option<(String, Option<f32>)> {
        match parse(text) {
            Some(Intent::Do { target, minutes }) => Some((target, minutes)),
            _ => None,
        }
    }

    fn bias(text: &str) -> Option<(String, f32)> {
        match parse(text) {
            Some(Intent::Bias { tag, weight }) => Some((tag, weight)),
            _ => None,
        }
    }

    #[test]
    fn 中文祈使句认得出来() {
        assert_eq!(go("去玩吧"), Some(("play".into(), None)));
        assert_eq!(go("你去玩会儿"), Some(("play".into(), None)));
        assert_eq!(go("快去休息一下"), Some(("rest".into(), None)));
        assert_eq!(go("休息一会儿吧"), Some(("rest".into(), None)));
        assert_eq!(go("该睡觉了"), Some(("sleep".into(), None)));
        assert_eq!(go("去工作吧！"), Some(("work".into(), None)));
        assert_eq!(go("你先去吃饭"), Some(("eat".into(), None)));
        assert_eq!(go("喝口水吧"), Some(("drink".into(), None)));
        assert_eq!(go("我想让你去学习"), Some(("study".into(), None)));
        assert_eq!(go("去午睡吧"), Some(("sleep".into(), None)));
        assert_eq!(go("玩去吧"), Some(("play".into(), None)));
    }

    #[test]
    fn 带时长() {
        assert_eq!(go("去玩 20 分钟"), Some(("play".into(), Some(20.0))));
        assert_eq!(go("玩半小时吧"), Some(("play".into(), Some(30.0))));
        assert_eq!(go("你去休息十分钟"), Some(("rest".into(), Some(10.0))));
        assert_eq!(go("去学习一个小时"), Some(("study".into(), Some(60.0))));
        assert_eq!(go("工作两个小时吧"), Some(("work".into(), Some(120.0))));
        assert_eq!(go("睡一个半小时"), Some(("sleep".into(), Some(90.0))));
        assert_eq!(go("go play for 15 minutes"), Some(("play".into(), Some(15.0))));
        assert_eq!(go("take a rest for half an hour"), Some(("rest".into(), Some(30.0))));
    }

    #[test]
    fn 闲聊不会被当成命令() {
        for text in [
            "我今天去玩了", "我在工作", "我想休息一下", "你昨天玩了什么", "你吃过饭了吗", "你喜欢玩游戏吗",
            "今天工作好累", "我学习了一下午", "你在干嘛", "你去玩了吗", "工作和学习哪个重要",
            "我们一起去玩过山车吧好不好呀我觉得会很好玩的你说呢", "你觉得睡觉重要吗", "你去哪儿玩了",
            "你去学校了吗", "你玩什么游戏", "你在玩什么", "吃饭了", "你会不会玩这个", "我们一起去玩了一天",
            "i played all day", "what did you eat", "did you sleep well", "i want to play", "what are you doing",
        ] {
            assert_eq!(go(text), None, "「{text}」被当成命令了");
        }
    }

    #[test]
    fn 带招呼语和后半句的命令() {
        assert_eq!(go("晚安，睡觉去吧"), Some(("sleep".into(), None)));
        assert_eq!(go("好了，你快去睡觉"), Some(("sleep".into(), None)));
        assert_eq!(go("你先去玩，我等会儿找你"), Some(("play".into(), None)));
        assert_eq!(go("我们一起去玩吧"), Some(("play".into(), None)));
        assert_eq!(go("快去吃饭了"), Some(("eat".into(), None)));
        assert_eq!(go("你去休息吧，我要睡了"), Some(("rest".into(), None)));
    }

    #[test]
    fn 英文祈使句() {
        assert_eq!(go("go play"), Some(("play".into(), None)));
        assert_eq!(go("Take a nap."), Some(("sleep".into(), None)));
        assert_eq!(go("You should rest now"), Some(("rest".into(), None)));
        assert_eq!(go("please go to work"), Some(("work".into(), None)));
        assert_eq!(go("time to study!"), Some(("study".into(), None)));
        assert_eq!(go("go eat something"), Some(("eat".into(), None)));
        assert_eq!(go("drink some water"), Some(("drink".into(), None)));
        assert_eq!(go("could you go play a game"), Some(("play".into(), None)));
    }

    #[test]
    fn 持续倾向() {
        assert_eq!(bias("多工作一点"), Some(("work".into(), 1.0)));
        assert_eq!(bias("少玩点"), Some(("play".into(), -1.0)));
        assert_eq!(bias("别玩了"), Some(("play".into(), -1.0)));
        assert_eq!(bias("你要多学习"), Some(("study".into(), 1.0)));
        assert_eq!(bias("不要工作了"), Some(("work".into(), -1.0)));
        assert_eq!(bias("work more"), Some(("work".into(), 1.0)));
        assert_eq!(bias("stop playing"), Some(("play".into(), -1.0)));
        // 吃喝睡不在可调之列
        assert_eq!(parse("少吃点"), None);
        assert_eq!(parse("多睡会儿"), None);
        // 「别玩了」不是「去玩」
        assert_eq!(go("别玩了"), None);
    }

    #[test]
    fn 番茄钟和提醒() {
        assert_eq!(parse("帮我设个番茄钟，学习一个小时吧"), Some(Intent::Focus { minutes: 60.0, target: Some("study".into()) }));
        assert_eq!(parse("来个番茄钟"), Some(Intent::Focus { minutes: DEFAULT_FOCUS_MIN, target: None }));
        assert_eq!(parse("专注工作 50 分钟"), Some(Intent::Focus { minutes: 50.0, target: Some("work".into()) }));
        assert_eq!(parse("start a 30 minute pomodoro"), Some(Intent::Focus { minutes: 30.0, target: None }));
        assert_eq!(parse("十分钟后叫我"), Some(Intent::Timer { minutes: 10.0, label: "提醒".into() }));
        assert_eq!(parse("提醒我 20 分钟后喝水"), Some(Intent::Timer { minutes: 20.0, label: "喝水".into() }));
        assert_eq!(parse("半小时后提醒我去接人"), Some(Intent::Timer { minutes: 30.0, label: "去接人".into() }));
        assert_eq!(parse("remind me in 15 minutes to stretch"), Some(Intent::Timer { minutes: 15.0, label: "stretch".into() }));
        // 没说多久的提醒认不出来，交给闲聊
        assert_eq!(parse("提醒我一下"), None);
        // 「去学习一小时」还是使唤，不是番茄钟
        assert_eq!(parse("去学习一个小时"), Some(Intent::Do { target: "study".into(), minutes: Some(60.0) }));
    }

    #[test]
    fn 中文数字() {
        assert_eq!(read_number_zh(&"十".chars().collect::<Vec<_>>()), (10.0, 1));
        assert_eq!(read_number_zh(&"二十五".chars().collect::<Vec<_>>()), (25.0, 3));
        assert_eq!(read_number_zh(&"两".chars().collect::<Vec<_>>()), (2.0, 1));
        assert_eq!(read_number_zh(&"15分".chars().collect::<Vec<_>>()), (15.0, 2));
        assert_eq!(read_number_zh(&"abc".chars().collect::<Vec<_>>()), (0.0, 0));
    }

    #[test]
    fn 空的和太长的直接放过() {
        assert_eq!(parse(""), None);
        assert_eq!(parse(&"去玩".repeat(40)), None);
    }
}
