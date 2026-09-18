//! 你在不在放歌，以及替你放歌。只认 Spotify：
//!
//! - **在不在播**：Spotify 播放时主窗口标题是「歌手 - 歌名」，暂停 / 没放时是「Spotify」
//!   或「Spotify Premium」。枚举顶层窗口，找进程是 `Spotify.exe` 的那个看标题——不用碰
//!   音频会话 COM，也不读别的窗口的内容。
//! - **放歌**：打开 `spotify:` 协议（商店版 / 桌面版都认），等它起来后如果还没在播，
//!   按一下键盘的「播放 / 暂停」媒体键。Spotify 会接着上次的列表放。
//!
//! 状态机只收到一个布尔（`Event::Music`），lib.rs 每十秒看一次；「放歌」意图先乐观地
//! 置真，看见它真在播就交给检测——你一按暂停她十秒内就停。

use std::sync::atomic::{AtomicI64, Ordering};

use tauri::AppHandle;

/// 「放歌」之后先当作在播这么久（ms）：Spotify 冷启动加缓冲要十几秒，别刚打开就说歌停了
const OPTIMISTIC_MS: i64 = 120_000;

static FORCED_UNTIL: AtomicI64 = AtomicI64::new(0);

/// 现在有没有歌在放。「放歌」之后的乐观期只用来撑过 Spotify 冷启动：
/// 一旦看见它真的在播，乐观期就结束，之后完全以窗口标题为准——你一按暂停她就该停
pub fn playing(now_ms: i64) -> bool {
    let state = spotify_state();
    if now_ms < FORCED_UNTIL.load(Ordering::Relaxed) {
        if state == Some(SpotifyState::Playing) {
            FORCED_UNTIL.store(0, Ordering::Relaxed);
        }
        return true;
    }
    state == Some(SpotifyState::Playing)
}

/* --- 听声音：第二道确认 + 高潮 --- */

/// 峰值低于这个算没声（0–1）。Spotify Launcher 挂着、标题却像在播的时候，这里是 0
const SILENT_PEAK: f32 = 0.004;
/// 标题说在播、但连着这么久没声，就当没在放（静音 / 假窗口）
pub const SILENT_GRACE_SEC: u32 = 20;
/// 高潮：短时电平 ≥ 长时均值的这么多倍，且绝对值够响；退出用更低的门槛，免得一句里抖来抖去
const CLIMAX_RATIO_ON: f32 = 1.35;
const CLIMAX_RATIO_OFF: f32 = 1.1;
const CLIMAX_MIN_PEAK: f32 = 0.35;
/// 长时均值至少这么大才谈得上高潮（刚开始放、还在淡入时不算）
const CLIMAX_MIN_BASE: f32 = 0.06;
/// 短时 / 长时 EMA 的系数（每秒采一次：快的约 2 秒，慢的约 45 秒）
const EMA_FAST: f32 = 0.5;
const EMA_SLOW: f32 = 0.022;

/// 电平表：每秒喂一个峰值进来
#[derive(Debug, Default, Clone)]
pub struct Meter {
    pub fast: f32,
    pub slow: f32,
    pub silent_for_sec: u32,
    pub climax: bool,
    samples: u32,
}

impl Meter {
    pub fn sample(&mut self, peak: f32) {
        self.samples += 1;
        if self.samples == 1 {
            self.fast = peak;
            self.slow = peak;
        } else {
            self.fast += (peak - self.fast) * EMA_FAST;
            self.slow += (peak - self.slow) * EMA_SLOW;
        }
        if peak < SILENT_PEAK {
            self.silent_for_sec += 1;
        } else {
            self.silent_for_sec = 0;
        }
        let ratio = if self.slow > 0.0 { self.fast / self.slow } else { 0.0 };
        self.climax = if self.climax {
            self.fast >= CLIMAX_MIN_PEAK * 0.8 && ratio >= CLIMAX_RATIO_OFF
        } else {
            self.slow >= CLIMAX_MIN_BASE && self.fast >= CLIMAX_MIN_PEAK && ratio >= CLIMAX_RATIO_ON
        };
    }

    /// 标题说在播，但声音这边不同意
    pub fn silent(&self) -> bool {
        self.silent_for_sec >= SILENT_GRACE_SEC
    }

    pub fn reset(&mut self) {
        *self = Meter::default();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyState {
    Playing,
    Idle,
}

/// 找到 Spotify 的主窗口看标题。没开就是 None
#[cfg(target_os = "windows")]
pub fn spotify_state() -> Option<SpotifyState> {
    use std::ffi::c_void;
    type Hwnd = *mut c_void;
    type Handle = *mut c_void;
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        fn IsWindowVisible(h: Hwnd) -> i32;
        fn GetWindowTextW(h: Hwnd, buf: *mut u16, n: i32) -> i32;
        fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(h: Handle) -> i32;
        fn QueryFullProcessImageNameW(h: Handle, flags: u32, buf: *mut u16, n: *mut u32) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    struct Found {
        titles: Vec<String>,
    }

    extern "system" fn visit(h: Hwnd, lparam: isize) -> i32 {
        // SAFETY: lparam 是我们自己传进来的 &mut Found；所有缓冲区都按 Win32 约定给足长度
        unsafe {
            let found = &mut *(lparam as *mut Found);
            if IsWindowVisible(h) == 0 {
                return 1;
            }
            let mut title = [0u16; 256];
            let n = GetWindowTextW(h, title.as_mut_ptr(), title.len() as i32);
            if n <= 0 {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, &mut pid);
            let proc_ = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if proc_.is_null() {
                return 1;
            }
            let mut path = [0u16; 1024];
            let mut len = path.len() as u32;
            let ok = QueryFullProcessImageNameW(proc_, 0, path.as_mut_ptr(), &mut len);
            CloseHandle(proc_);
            if ok == 0 {
                return 1;
            }
            let exe = String::from_utf16_lossy(&path[..len as usize]).to_lowercase();
            if exe.ends_with("\\spotify.exe") {
                found.titles.push(String::from_utf16_lossy(&title[..n as usize]));
            }
            1
        }
    }

    let mut found = Found { titles: Vec::new() };
    // SAFETY: 回调只在 EnumWindows 返回前被调用，found 在此期间一直活着
    unsafe {
        EnumWindows(visit, &mut found as *mut Found as isize);
    }
    if found.titles.is_empty() {
        return None;
    }
    Some(if found.titles.iter().any(|t| title_means_playing(t)) { SpotifyState::Playing } else { SpotifyState::Idle })
}

#[cfg(not(target_os = "windows"))]
pub fn spotify_state() -> Option<SpotifyState> {
    None
}

/// 「歌手 - 歌名」算在播；「Spotify」「Spotify Premium」「Spotify Free」是没在播
pub fn title_means_playing(title: &str) -> bool {
    let t = title.trim();
    !t.is_empty() && !t.starts_with("Spotify") && t.contains(" - ")
}

/// 「放首歌」：打开 Spotify，没在播就按一下媒体播放键。异步等它起来，不卡对话
pub fn play_via_spotify(app: &AppHandle) -> Result<(), String> {
    let _ = app;
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", "spotify:"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("打不开 Spotify：{e}"))?;
        FORCED_UNTIL.store(crate::now_ms() + OPTIMISTIC_MS, Ordering::Relaxed);
        std::thread::spawn(|| {
            // 等窗口出现；已经在播就什么都不做，否则按一下播放键
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                match spotify_state() {
                    Some(SpotifyState::Playing) => return,
                    Some(SpotifyState::Idle) => {
                        std::thread::sleep(std::time::Duration::from_millis(1500));
                        if spotify_state() == Some(SpotifyState::Idle) {
                            press_media_play();
                        }
                        return;
                    }
                    None => {}
                }
            }
            log::warn!("Spotify 十秒内没起来，没按播放键");
        });
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("只在 Windows 上支持".into())
    }
}

/// 键盘上的「播放 / 暂停」媒体键（VK_MEDIA_PLAY_PAUSE）
#[cfg(target_os = "windows")]
fn press_media_play() {
    #[link(name = "user32")]
    extern "system" {
        fn keybd_event(vk: u8, scan: u8, flags: u32, extra: usize);
    }
    const VK_MEDIA_PLAY_PAUSE: u8 = 0xB3;
    const KEYEVENTF_KEYUP: u32 = 0x0002;
    // SAFETY: 纯 Win32 调用，参数按文档
    unsafe {
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 0, 0);
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, KEYEVENTF_KEYUP, 0);
    }
    log::info!("按了媒体播放键");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 电平表_没声就当没放_响起来算高潮() {
        let mut m = Meter::default();
        for _ in 0..SILENT_GRACE_SEC {
            m.sample(0.0);
        }
        assert!(m.silent(), "二十秒没声");
        m.sample(0.2);
        assert!(!m.silent(), "有声了就不算静");

        let mut m = Meter::default();
        for _ in 0..60 {
            m.sample(0.15); // 主歌：平稳
        }
        assert!(!m.climax);
        for _ in 0..4 {
            m.sample(0.6); // 副歌炸开
        }
        assert!(m.climax, "fast {} slow {}", m.fast, m.slow);
        for _ in 0..30 {
            m.sample(0.12); // 回落
        }
        assert!(!m.climax);
    }

    #[test]
    fn 标题看得出在不在播() {
        assert!(title_means_playing("YOASOBI - アイドル"));
        assert!(!title_means_playing("Spotify"));
        assert!(!title_means_playing("Spotify Premium"));
        assert!(!title_means_playing("Spotify Free"));
        assert!(!title_means_playing(""));
    }
}
