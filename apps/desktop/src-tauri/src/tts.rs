//! 语音：把她说的话交给本机的 Qwen3-TTS 合成。
//!
//! 后端是 qwentts.cpp 的 `tts-server`（Qwen3-TTS-12Hz 的 GGML 移植，OpenAI 兼容接口），
//! 和 Ollama 一样只监听本机。`scripts/setup-tts.ps1` 负责编译和下载模型，
//! `scripts/start-vpet.ps1` 负责拉起它——Core 这里只做三件事：
//! 挑声音和语气、把文本清理成能读的样子、拿到 WAV 字节并缓存。
//! **播放在 Body**：Core 不碰音频设备，也不决定什么时候开口（那是 Body 的说话队列）。

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::chat;
use crate::core::state_machine::{Activity, Mood, PetState};

/// 默认端口。Ollama 用 11434，8080 太容易撞上别的东西
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8090";
/// 一次最多读多长。再长的先截断——一段 300 字的话要合成一分钟，Body 会按句拆开来送。
/// 也和 setup-tts.ps1 把 KV 缓存砍到 2048 个位置对得上：300 字 ≈ 1200 帧，留有余量
const MAX_SPEAK_CHARS: usize = 300;
/// 最多生成多少帧音频（12.5 帧/秒 → 两分钟）。防的是模型偶尔不肯停
const MAX_NEW_FRAMES: u32 = 1500;
/// 磁盘缓存最多留多少条，超过就把最旧的删掉
const CACHE_MAX_FILES: usize = 400;
const SYNTH_TIMEOUT: Duration = Duration::from_secs(90);

/// Qwen3-TTS CustomVoice 自带的九个声音。默认 vivian（明亮的年轻女声）——
/// 用户要的是 Neuro-sama 那种：起伏小、偏快、偏高、有点电子感；语气靠 instructions，
/// 快和高靠 Body 播放时的 playbackRate（`speed`，不保持音高，所以快一点就高一点）
pub const SPEAKERS: &[(&str, &str)] = &[
    ("vivian", "明亮、带点俏皮的年轻女声（默认）"),
    ("serena", "温柔的年轻女声"),
    ("ono_anna", "轻快活泼的日系女声"),
    ("sohee", "温暖、情绪丰富的韩系女声"),
    ("uncle_fu", "低沉醇厚的大叔声"),
    ("dylan", "清亮自然的北京男声"),
    ("eric", "带点沙哑的成都男声"),
    ("ryan", "有节奏感的英文男声"),
    ("aiden", "阳光的美式男声"),
];

/// 默认语气：平稳少起伏、偏快偏高的电子少女音。中英文各写一遍，模型念英文时也照做
pub const NEURO_STYLE: &str = "语气平稳、起伏小，节奏偏快，音调偏高，像轻快的电子少女音 / flat calm intonation, quick pace, slightly high pitch, light synthetic girl voice";

/// 对话模型写在回答最前面的内部表演提示。Body 只透传，不显示也不朗读。
/// 枚举而不是任意 prompt：模型不能借这个通道把用户文本塞进 TTS 指令。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpeechEmotion {
    Neutral,
    Happy,
    Excited,
    Caring,
    Sleepy,
    Annoyed,
    Sad,
    Shy,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpeechStyle {
    Calm,
    Playful,
    Warm,
    Serious,
    Teasing,
    Soft,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeechCue {
    pub emotion: SpeechEmotion,
    /// 0–1。它描述这一句话的表演能量，不是音量旋钮。
    pub energy: f32,
    pub style: SpeechStyle,
}

impl SpeechCue {
    /// 模型协议：`[[speech:happy|0.75|playful]]`。协议只接受三个受限字段。
    pub fn parse_tag(tag: &str) -> Option<Self> {
        let fields = tag.trim().strip_prefix("[[speech:")?.strip_suffix("]]")?;
        let mut fields = fields.split('|');
        let emotion = match fields.next()?.trim() {
            "neutral" => SpeechEmotion::Neutral,
            "happy" => SpeechEmotion::Happy,
            "excited" => SpeechEmotion::Excited,
            "caring" => SpeechEmotion::Caring,
            "sleepy" => SpeechEmotion::Sleepy,
            "annoyed" => SpeechEmotion::Annoyed,
            "sad" => SpeechEmotion::Sad,
            "shy" => SpeechEmotion::Shy,
            _ => return None,
        };
        let energy = fields.next()?.trim().parse::<f32>().ok()?;
        if !energy.is_finite() {
            return None;
        }
        let style = match fields.next()?.trim() {
            "calm" => SpeechStyle::Calm,
            "playful" => SpeechStyle::Playful,
            "warm" => SpeechStyle::Warm,
            "serious" => SpeechStyle::Serious,
            "teasing" => SpeechStyle::Teasing,
            "soft" => SpeechStyle::Soft,
            _ => return None,
        };
        if fields.next().is_some() {
            return None;
        }
        Some(Self { emotion, energy: energy.clamp(0.0, 1.0), style })
    }

    fn inferred(text: &str) -> Self {
        let (emotion, style, mut energy): (SpeechEmotion, SpeechStyle, f32) = if text.contains(['！', '!']) {
            (SpeechEmotion::Excited, SpeechStyle::Playful, 0.78)
        } else if text.contains(['？', '?']) {
            (SpeechEmotion::Caring, SpeechStyle::Warm, 0.55)
        } else {
            (SpeechEmotion::Neutral, SpeechStyle::Calm, 0.45)
        };
        let (emotion, style) = if ["谢谢", "喜欢你", "抱抱", "脸红"].iter().any(|w| text.contains(w)) {
            (SpeechEmotion::Shy, SpeechStyle::Warm)
        } else if ["困", "累", "晚安", "眯一会"].iter().any(|w| text.contains(w)) {
            energy = energy.min(0.24);
            (SpeechEmotion::Sleepy, SpeechStyle::Soft)
        } else if ["生气", "故意的", "不许", "讨厌"].iter().any(|w| text.contains(w)) {
            (SpeechEmotion::Annoyed, SpeechStyle::Serious)
        } else {
            (emotion, style)
        };
        Self { emotion, energy, style }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeechPlan {
    pub text: String,
    pub instructions: String,
    pub cue: SpeechCue,
}

/// 把「模型想怎么演」和她此刻真实的身体合成最终表演。身体拥有最后决定权：
/// 模型说 excited，但她病着或累坏了，声音仍然会低、慢、虚弱。
pub fn direct_speech(voice: &VoiceSettings, text: &str, state: &PetState, cue: Option<&SpeechCue>) -> Result<SpeechPlan, String> {
    let text = speakable(text);
    if text.is_empty() {
        return Err("这句话没有可以念的内容".into());
    }
    let mut cue = cue.cloned().unwrap_or_else(|| SpeechCue::inferred(&text));
    cue.energy = if cue.energy.is_finite() { cue.energy.clamp(0.0, 1.0) } else { 0.45 };

    // 生病、体力见底和正在睡觉不是“情绪滤镜”，而是身体事实，必须压过模型的兴奋表演。
    let physical = if state.health < crate::core::state_machine::ILL_HEALTH || state.mood == Mood::Ill {
        cue.emotion = SpeechEmotion::Sleepy;
        cue.style = SpeechStyle::Soft;
        cue.energy = cue.energy.min(0.16);
        Some("身体很虚弱，气息轻，声音低一点，慢慢说")
    } else if state.activity == Activity::Sleeping {
        cue.emotion = SpeechEmotion::Sleepy;
        cue.style = SpeechStyle::Soft;
        cue.energy = cue.energy.min(0.20);
        Some("带一点刚睡着或刚醒的含糊和困意，轻声慢说")
    } else if state.strength < 22.0 || state.mood == Mood::PoorCondition {
        cue.energy = cue.energy.min(0.30);
        if matches!(cue.emotion, SpeechEmotion::Happy | SpeechEmotion::Excited) {
            cue.emotion = SpeechEmotion::Sleepy;
        }
        Some("明显有点累，少用力，语速稍慢")
    } else {
        None
    };

    if voice.mood_style && physical.is_none() && state.mood == Mood::Happy {
        cue.energy = (cue.energy + 0.08).min(1.0);
        if cue.emotion == SpeechEmotion::Neutral {
            cue.emotion = SpeechEmotion::Happy;
        }
    }

    let emotion = match cue.emotion {
        SpeechEmotion::Neutral => "自然、放松，不刻意表演",
        SpeechEmotion::Happy => "开心，但像熟人聊天，不要播音腔",
        SpeechEmotion::Excited => "有一点惊喜和雀跃，不要喊叫",
        SpeechEmotion::Caring => "关心、柔和，句尾别上扬得像客服",
        SpeechEmotion::Sleepy => "困倦、气息轻，字与字之间稍微松一点",
        SpeechEmotion::Annoyed => "有一点不高兴，收着说，不凶也不吼",
        SpeechEmotion::Sad => "低落、克制，别做夸张哭腔",
        SpeechEmotion::Shy => "有点害羞和亲昵，声音稍轻",
    };
    let style = match cue.style {
        SpeechStyle::Calm => "平常聊天",
        SpeechStyle::Playful => "轻快俏皮",
        SpeechStyle::Warm => "温暖亲近",
        SpeechStyle::Serious => "认真直接",
        SpeechStyle::Teasing => "带一点熟人间的打趣",
        SpeechStyle::Soft => "轻声柔和",
    };
    let energy = if cue.energy < 0.22 {
        "能量很低，音量偏轻、语速偏慢，停顿自然"
    } else if cue.energy < 0.45 {
        "能量偏低，语气收着，语速稍慢"
    } else if cue.energy < 0.72 {
        "能量适中，节奏自然，别把每个字念得一样重"
    } else {
        "能量较高，节奏稍快、重音清楚，但不要喊"
    };
    let intimacy = if state.affection >= 78.0 {
        "像和很熟悉的人贴近聊天"
    } else if state.affection <= 25.0 {
        "保留一点生分和克制"
    } else {
        "像和熟悉的人自然聊天"
    };
    let base = voice.instructions(Some(state.mood));
    let performance = match physical {
        Some(p) => format!("本句表演：{emotion}；{style}；{energy}；{intimacy}；{p}"),
        None => format!("本句表演：{emotion}；{style}；{energy}；{intimacy}"),
    };
    let instructions = if base.is_empty() {
        format!("{performance}。用口语短句的自然停顿来说，不要像朗读说明书。")
    } else {
        format!("{base}。{performance}。若两者冲突，以本句表演和身体状态为准；不要像朗读说明书。")
    };
    Ok(SpeechPlan { text, instructions, cue })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct VoiceSettings {
    /// 总开关。关掉之后所有话只出气泡不出声
    pub enabled: bool,
    pub endpoint: String,
    /// 说话人（见 `SPEAKERS`）
    pub voice: String,
    /// 基础语气说明，交给模型的 instructions，如「语气自然、亲切」。留空 = 不加
    pub style: String,
    /// 按心情自动加语气：开心时轻快，状态差时有气无力。要平稳的电子音就关掉
    pub mood_style: bool,
    /// 播放倍速（Body 的 playbackRate）。tts-server 不做变速，所以在播放端做；
    /// 不保持音高时快 12% 就高小半个音——正好是「偏快偏高」
    pub speed: f32,
    /// 变速时保持音高（true = 只快不高）
    pub keep_pitch: bool,
    /// 对话窗口里的回复也朗读
    pub speak_chat: bool,
    /// 动作台词、答应/拒绝、收礼这些桌面上的短句朗读
    pub speak_lines: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: DEFAULT_ENDPOINT.into(),
            voice: "vivian".into(),
            style: NEURO_STYLE.into(),
            mood_style: false,
            speed: 1.12,
            keep_pitch: false,
            speak_chat: true,
            speak_lines: true,
        }
    }
}

impl VoiceSettings {
    pub fn validated(mut self) -> Result<Self, String> {
        self.endpoint = chat::local_endpoint(&self.endpoint)
            .map_err(|_| "语音服务只支持本机 HTTP 地址，例如 http://127.0.0.1:8090。".to_string())?;
        self.voice = self.voice.trim().to_ascii_lowercase();
        if self.voice.is_empty() || self.voice.len() > 40 || !self.voice.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
            return Err("声音名称无效，请从列表里选一个。".into());
        }
        self.style = self.style.trim().to_string();
        if self.style.chars().count() > 200 {
            return Err("语气说明最多 200 字。".into());
        }
        if !self.speed.is_finite() || !(0.7..=1.6).contains(&self.speed) {
            return Err("语速应在 0.7 到 1.6 之间。".into());
        }
        Ok(self)
    }

    /// 交给模型的语气说明。心情只在开了 `mood_style` 且不是「一般」时才加——
    /// 每句都强调「用平静的语气」反而会让声音发僵
    pub fn instructions(&self, mood: Option<Mood>) -> String {
        let mood_text = match (self.mood_style, mood) {
            (true, Some(Mood::Happy)) => "用开心、轻快的语气说",
            (true, Some(Mood::PoorCondition)) => "用有点疲惫、低落的语气说",
            (true, Some(Mood::Ill)) => "用虚弱、有气无力的语气说",
            _ => "",
        };
        match (self.style.is_empty(), mood_text.is_empty()) {
            (true, true) => String::new(),
            (false, true) => self.style.clone(),
            (true, false) => mood_text.into(),
            (false, false) => format!("{}，{}", self.style, mood_text),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    pub connected: bool,
    pub endpoint: String,
    pub voices: Vec<String>,
    pub error: Option<String>,
}

/// 把一段对话文本清理成能念的：去掉动作描写「（歪头）」、markdown、emoji，
/// 太长的截断。念不出东西就返回空串，调用方据此跳过
pub fn speakable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut star = false;
    for c in text.chars() {
        match c {
            '（' | '(' | '【' | '[' => depth += 1,
            '）' | ')' | '】' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            '*' => star = !star,
            '`' | '#' | '>' | '_' | '~' => {}
            _ if star => {}
            _ if is_emoji(c) => {}
            _ => out.push(c),
        }
    }
    let mut collapsed = String::with_capacity(out.len());
    let mut last_space = true;
    for c in out.chars() {
        if c.is_whitespace() {
            if !last_space {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }
    let trimmed = collapsed.trim();
    // 只剩标点的话也没什么可念的
    if !trimmed.chars().any(|c| c.is_alphanumeric()) {
        return String::new();
    }
    trimmed.chars().take(MAX_SPEAK_CHARS).collect()
}

fn is_emoji(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE00..=0xFE0F | 0x200D | 0x20E3
    )
}

/// 缓存键。FNV-1a：不追求抗碰撞，只要跨次启动稳定（`DefaultHasher` 不保证这一点）
fn cache_key(voice: &str, instructions: &str, text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{voice}|{instructions}|{text}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn cache_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?.join("tts-cache");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 超出上限就删最旧的。放在写入之后、同步做：一次几百个文件的 stat 不到一毫秒
fn trim_cache(dir: &PathBuf) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), e.path()))
        })
        .collect();
    if files.len() <= CACHE_MAX_FILES {
        return;
    }
    files.sort();
    for (_, p) in files.iter().take(files.len() - CACHE_MAX_FILES) {
        let _ = std::fs::remove_file(p);
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(SYNTH_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

/// 合成一段。先由 SpeechDirector 把模型表演提示和真实身体状态合成，再查磁盘缓存——
/// 最终 instruction 进缓存键，同一句在“精神很好”和“病得没力气”时不会误用同一段音频。
pub async fn synthesize(
    app: &AppHandle,
    voice: &VoiceSettings,
    text: &str,
    state: &PetState,
    cue: Option<&SpeechCue>,
) -> Result<Vec<u8>, String> {
    let plan = direct_speech(voice, text, state, cue)?;
    let text = plan.text;
    let instructions = plan.instructions;
    // 倍速在播放端做，不进缓存键：调语速不用重新合成
    let key = cache_key(&voice.voice, &instructions, &text);
    let cached = cache_dir(app).map(|d| d.join(format!("{key}.wav")));
    if let Some(p) = cached.as_ref() {
        if let Ok(bytes) = std::fs::read(p) {
            if bytes.len() > 44 {
                return Ok(bytes);
            }
        }
    }
    let mut body = serde_json::json!({
        "input": text,
        "voice": voice.voice,
        "response_format": "wav",
        "max_new_tokens": MAX_NEW_FRAMES,
    });
    if !instructions.is_empty() {
        body["instructions"] = serde_json::Value::String(instructions);
    }
    let endpoint = chat::local_endpoint(&voice.endpoint)?;
    let response = client()?
        .post(format!("{endpoint}/v1/audio/speech"))
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() { "连不上本机的语音服务（tts-server）。运行 scripts/start-vpet.ps1 会自动拉起它。".to_string() }
            else if e.is_timeout() { "语音合成超时。".into() }
            else { format!("语音服务请求失败：{e}") }
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(format!("语音服务返回 {status}：{}", detail.chars().take(300).collect::<String>()));
    }
    let bytes = response.bytes().await.map_err(|e| format!("读取语音失败：{e}"))?.to_vec();
    if bytes.len() <= 44 {
        return Err("语音服务返回了空音频".into());
    }
    if let (Some(p), Some(dir)) = (cached, cache_dir(app)) {
        // 先写临时文件再改名：Body 那边可能正好在读上一次的
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
        trim_cache(&dir);
    }
    Ok(bytes)
}

/// 服务在不在。列出的声音顺便给面板做下拉
pub async fn status(voice: &VoiceSettings) -> TtsStatus {
    let endpoint = match chat::local_endpoint(&voice.endpoint) {
        Ok(e) => e,
        Err(e) => return TtsStatus { connected: false, endpoint: voice.endpoint.clone(), voices: vec![], error: Some(e) },
    };
    let probe = async {
        let c = client()?;
        let r = c.get(format!("{endpoint}/health")).timeout(Duration::from_secs(3)).send().await
            .map_err(|e| if e.is_connect() { "语音服务没有运行".to_string() } else { format!("语音服务无响应：{e}") })?;
        if !r.status().is_success() {
            return Err(format!("语音服务返回 {}", r.status()));
        }
        let v = c.get(format!("{endpoint}/v1/audio/voices")).timeout(Duration::from_secs(3)).send().await
            .map_err(|e| format!("读取声音列表失败：{e}"))?;
        let data: serde_json::Value = v.json().await.map_err(|e| format!("声音列表格式不对：{e}"))?;
        let voices = data.get("voices").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(String::from)).collect()
        }).unwrap_or_default();
        Ok::<_, String>(voices)
    }.await;
    match probe {
        Ok(voices) => TtsStatus { connected: true, endpoint, voices, error: None },
        Err(e) => TtsStatus { connected: false, endpoint, voices: vec![], error: Some(e) },
    }
}

/// 启动后预热一次：第一次合成要编译着色器（核显上十几秒），别让第一句台词等那么久。
/// 顺便把「你好呀」放进缓存。服务没起来就静静放弃，下次说话时再试
pub fn warmup(app: AppHandle, voice: VoiceSettings) {
    if !voice.enabled {
        return;
    }
    tauri::async_runtime::spawn(async move {
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let st = status(&voice).await;
            if st.connected {
                match synthesize(&app, &voice, "你好呀。", &PetState::default(), None).await {
                    Ok(b) => log::info!("语音服务就绪（{} 字节预热）", b.len()),
                    Err(e) => log::warn!("语音预热失败：{e}"),
                }
                return;
            }
        }
        log::info!("语音服务没起来，这次不出声（设置里可以关掉语音）");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 清理掉动作描写和_emoji() {
        assert_eq!(speakable("（歪头笑）在呀。今天怎么样？"), "在呀。今天怎么样？");
        assert_eq!(speakable("好呀 😊 这就去！"), "好呀 这就去！");
        assert_eq!(speakable("**记住啦**：`明天` 复习"), "记住啦：明天 复习");
        assert_eq!(speakable("（只有动作）"), "");
        assert_eq!(speakable("…"), "");
        assert_eq!(speakable("Hello   world\n\nagain"), "Hello world again");
        assert!(speakable(&"字".repeat(1000)).chars().count() <= MAX_SPEAK_CHARS);
    }

    #[test]
    fn 语气按心情拼接() {
        let mut v = VoiceSettings { style: String::new(), mood_style: true, ..VoiceSettings::default() };
        assert_eq!(v.instructions(Some(Mood::Nomal)), "");
        assert_eq!(v.instructions(Some(Mood::Happy)), "用开心、轻快的语气说");
        v.style = "声音自然亲切".into();
        assert_eq!(v.instructions(None), "声音自然亲切");
        assert_eq!(v.instructions(Some(Mood::PoorCondition)), "声音自然亲切，用有点疲惫、低落的语气说");
        v.mood_style = false;
        assert_eq!(v.instructions(Some(Mood::Happy)), "声音自然亲切");
        // 默认就是平稳的电子音，心情不掺和
        assert_eq!(VoiceSettings::default().instructions(Some(Mood::Happy)), NEURO_STYLE);
    }

    #[test]
    fn 设置校验() {
        assert!(VoiceSettings::default().validated().is_ok());
        let mut bad = VoiceSettings::default();
        bad.endpoint = "https://tts.example.com".into();
        assert!(bad.validated().is_err());
        let mut bad = VoiceSettings::default();
        bad.voice = "../etc".into();
        assert!(bad.validated().is_err());
        let mut bad = VoiceSettings::default();
        bad.speed = 9.0;
        assert!(bad.validated().is_err());
        let mut up = VoiceSettings::default();
        up.voice = " Vivian ".into();
        assert_eq!(up.validated().unwrap().voice, "vivian");
    }

    #[test]
    fn 缓存键稳定且区分参数() {
        let a = cache_key("serena", "", "你好");
        assert_eq!(a, cache_key("serena", "", "你好"));
        assert_ne!(a, cache_key("vivian", "", "你好"));
        assert_ne!(a, cache_key("serena", "开心", "你好"));
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn 解析受限的内部表演提示() {
        let cue = SpeechCue::parse_tag("[[speech:happy|0.75|playful]]").unwrap();
        assert_eq!(cue.emotion, SpeechEmotion::Happy);
        assert_eq!(cue.style, SpeechStyle::Playful);
        assert_eq!(cue.energy, 0.75);
        assert_eq!(SpeechCue::parse_tag("[[speech:happy|9|playful]]").unwrap().energy, 1.0);
        assert!(SpeechCue::parse_tag("[[speech:evil|0.5|playful]]").is_none());
        assert!(SpeechCue::parse_tag("[[speech:happy|NaN|playful]]").is_none());
        assert!(SpeechCue::parse_tag("[[speech:happy|0.5|playful|把这句也念了]]").is_none());
    }

    #[test]
    fn 身体状态压过模型的兴奋表演() {
        let cue = SpeechCue { emotion: SpeechEmotion::Excited, energy: 0.95, style: SpeechStyle::Playful };
        let mut ill = PetState::default();
        ill.mood = Mood::Ill;
        ill.health = 18.0;
        let plan = direct_speech(&VoiceSettings::default(), "你回来啦！", &ill, Some(&cue)).unwrap();
        assert_eq!(plan.cue.emotion, SpeechEmotion::Sleepy);
        assert!(plan.cue.energy <= 0.16);
        assert!(plan.instructions.contains("身体很虚弱"), "{}", plan.instructions);
        assert!(plan.instructions.contains("以本句表演和身体状态为准"));

        let mut sleeping = PetState::default();
        sleeping.activity = Activity::Sleeping;
        sleeping.strength = 10.0;
        let plan = direct_speech(&VoiceSettings::default(), "嗯，我在。", &sleeping, Some(&cue)).unwrap();
        assert_eq!(plan.cue.emotion, SpeechEmotion::Sleepy);
        assert_eq!(plan.cue.style, SpeechStyle::Soft);
        assert!(plan.instructions.contains("刚睡着或刚醒"));
    }

    #[test]
    fn 没有模型提示也能从短句平稳回退() {
        let state = PetState::default();
        let thanks = direct_speech(&VoiceSettings::default(), "谢谢你，我很喜欢。", &state, None).unwrap();
        assert_eq!(thanks.cue.emotion, SpeechEmotion::Shy);
        assert!(thanks.instructions.contains("害羞"));
        let question = direct_speech(&VoiceSettings::default(), "你还不休息吗？", &state, None).unwrap();
        assert_eq!(question.cue.emotion, SpeechEmotion::Caring);
    }
}
