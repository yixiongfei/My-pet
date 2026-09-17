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
use crate::core::state_machine::Mood;

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

/// Qwen3-TTS CustomVoice 自带的九个声音。萝莉斯是个温柔又好奇的少女，
/// 默认用 serena（温暖柔和的年轻女声，中英文都行）；想活泼一点换 vivian
pub const SPEAKERS: &[(&str, &str)] = &[
    ("serena", "温柔的年轻女声（默认）"),
    ("vivian", "明亮、带点俏皮的年轻女声"),
    ("ono_anna", "轻快活泼的日系女声"),
    ("sohee", "温暖、情绪丰富的韩系女声"),
    ("uncle_fu", "低沉醇厚的大叔声"),
    ("dylan", "清亮自然的北京男声"),
    ("eric", "带点沙哑的成都男声"),
    ("ryan", "有节奏感的英文男声"),
    ("aiden", "阳光的美式男声"),
];

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
    /// 按心情自动加语气：开心时轻快，状态差时有气无力
    pub mood_style: bool,
    /// 语速倍率
    pub speed: f32,
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
            voice: "serena".into(),
            style: String::new(),
            mood_style: true,
            speed: 1.0,
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
        if !self.speed.is_finite() || !(0.5..=2.0).contains(&self.speed) {
            return Err("语速应在 0.5 到 2.0 之间。".into());
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
fn cache_key(voice: &str, instructions: &str, speed: f32, text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{voice}|{instructions}|{speed:.2}|{text}").bytes() {
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

/// 合成一段。先查磁盘缓存——固定台词第二次说不该再等两秒
pub async fn synthesize(app: &AppHandle, voice: &VoiceSettings, text: &str, mood: Option<Mood>) -> Result<Vec<u8>, String> {
    let text = speakable(text);
    if text.is_empty() {
        return Err("这句话没有可以念的内容".into());
    }
    let instructions = voice.instructions(mood);
    let key = cache_key(&voice.voice, &instructions, voice.speed, &text);
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
        "speed": voice.speed,
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
                match synthesize(&app, &voice, "你好呀。", None).await {
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
        let mut v = VoiceSettings::default();
        assert_eq!(v.instructions(Some(Mood::Nomal)), "");
        assert_eq!(v.instructions(Some(Mood::Happy)), "用开心、轻快的语气说");
        v.style = "声音自然亲切".into();
        assert_eq!(v.instructions(None), "声音自然亲切");
        assert_eq!(v.instructions(Some(Mood::PoorCondition)), "声音自然亲切，用有点疲惫、低落的语气说");
        v.mood_style = false;
        assert_eq!(v.instructions(Some(Mood::Happy)), "声音自然亲切");
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
        let a = cache_key("serena", "", 1.0, "你好");
        assert_eq!(a, cache_key("serena", "", 1.0, "你好"));
        assert_ne!(a, cache_key("vivian", "", 1.0, "你好"));
        assert_ne!(a, cache_key("serena", "开心", 1.0, "你好"));
        assert_ne!(a, cache_key("serena", "", 1.2, "你好"));
        assert_eq!(a.len(), 16);
    }
}
