//! 宠物窗口的自主移动与贴边几何。
//!
//! 动画仍由 Body 播，Core 只负责规则选择、窗口位置和模式事件。所有原版坐标
//! 都以底部 500×500 的身体画布为基准；窗口上方的气泡区不参与距离和锚点计算。

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

use crate::{roll, PET_WINDOW};

const PET_LOGICAL_SIZE: f64 = 500.0;
const SIDE_TRIGGER: f64 = 50.0;

const LEFT: u16 = 1;
const RIGHT: u16 = 2;
const TOP: u16 = 4;
const BOTTOM: u16 = 8;
const LEFT_GREATER: u16 = 16;
const RIGHT_GREATER: u16 = 32;
const TOP_GREATER: u16 = 64;
const BOTTOM_GREATER: u16 = 128;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum LocateType {
    None,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveRule {
    pub graph: String,
    pub locate_type: LocateType,
    pub locate_length: f64,
    pub interval: u64,
    pub check_type: u16,
    pub check_left: f64,
    pub check_right: f64,
    pub check_top: f64,
    pub check_bottom: f64,
    pub mode_type: u8,
    pub trigger_type: u16,
    pub trigger_left: f64,
    pub trigger_right: f64,
    pub trigger_top: f64,
    pub trigger_bottom: f64,
    pub speed_x: f64,
    pub speed_y: f64,
    pub distance: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SideAnchors {
    pub left: f64,
    pub right: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionProfile {
    pub moves: Vec<MoveRule>,
    pub side: SideAnchors,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
struct ScreenBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug)]
struct ActiveMove {
    id: u64,
    rule_index: usize,
    mood_bit: u8,
    bounds: ScreenBounds,
    stepping: bool,
    next_step: Instant,
    walk_length: u32,
}

#[derive(Default)]
pub struct PetMotion {
    profile: Option<MotionProfile>,
    side: Option<Side>,
    active: Option<ActiveMove>,
    next_id: u64,
}

pub type MotionState = Mutex<PetMotion>;

/// 同一个枚举既是命令返回值，也是 Core 因碰到边界主动推给 Body 的事件。
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum MotionEvent {
    #[serde(rename = "side")]
    Side { side: Side },
    #[serde(rename = "side-stop")]
    SideStop,
    #[serde(rename = "move")]
    Move { id: u64, graph: String },
    #[serde(rename = "move-continue")]
    MoveContinue { id: u64 },
    #[serde(rename = "move-stop")]
    MoveStop { id: u64, graph: String },
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    monitor_height: u32,
}

#[derive(Clone, Copy, Debug)]
struct Distances {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

#[derive(Clone, Copy)]
struct Limits {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

impl Geometry {
    fn scale(self) -> f64 {
        self.width.max(1) as f64 / PET_LOGICAL_SIZE
    }

    fn headroom(self) -> i32 {
        self.height.saturating_sub(self.width) as i32
    }

    fn monitor_right(self) -> f64 {
        self.monitor_x as f64 + self.monitor_width as f64
    }

    fn monitor_bottom(self) -> f64 {
        self.monitor_y as f64 + self.monitor_height as f64
    }

    /// 原版的四个 distance 是人物方画布到当前活动屏幕四边的有符号距离。
    fn distances(self) -> Distances {
        Distances {
            left: self.x as f64 - self.monitor_x as f64,
            right: self.monitor_right() - (self.x as f64 + self.width as f64),
            top: (self.y + self.headroom()) as f64 - self.monitor_y as f64,
            bottom: self.monitor_bottom() - (self.y as f64 + self.height as f64),
        }
    }

    fn screen_bounds(self) -> ScreenBounds {
        ScreenBounds {
            x: self.monitor_x,
            y: self.monitor_y,
            width: self.monitor_width,
            height: self.monitor_height,
        }
    }

    fn clamped_y(self) -> i32 {
        // 身体顶 = window y + (height - width)，身体底 = window y + height。
        let min = self.monitor_y - self.headroom();
        let max = self.monitor_y + self.monitor_height as i32 - self.height as i32;
        if min <= max {
            self.y.clamp(min, max)
        } else {
            // 身体本身比屏幕还高时没有同时满足上下边界的位置，优先保证头部可见。
            min
        }
    }
}

#[tauri::command]
pub fn set_pet_motion_profile(state: tauri::State<'_, MotionState>, profile: MotionProfile) {
    log::info!("收到移动规则：{} 条", profile.moves.len());
    if let Ok(mut motion) = state.lock() {
        motion.profile = Some(profile);
        motion.active = None;
    }
}

/// Body 真正空闲时调用。Core 按原版 TriggerType / ModeType 从 16 条规则中随机挑一条。
#[tauri::command]
pub fn start_pet_motion(
    app: AppHandle,
    state: tauri::State<'_, MotionState>,
    mood: String,
) -> Option<MotionEvent> {
    let win = app.get_webview_window(PET_WINDOW)?;
    let geometry = geometry(&app, &win)?;
    let mood_bit = mood_bit(&mood)?;
    let mut motion = state.lock().ok()?;
    if motion.side.is_some() || motion.active.is_some() {
        return None;
    }
    let candidates: Vec<usize> = motion
        .profile
        .as_ref()?
        .moves
        .iter()
        .enumerate()
        .filter_map(|(i, rule)| triggered(rule, geometry, mood_bit).then_some(i))
        .collect();
    let rule_index = *candidates.get(random_index(candidates.len())?)?;
    let graph = motion.profile.as_ref()?.moves[rule_index].graph.clone();
    let id = next_id(&mut motion);
    motion.active = Some(ActiveMove {
        id,
        rule_index,
        mood_bit,
        bounds: geometry.screen_bounds(),
        stepping: false,
        next_step: Instant::now(),
        walk_length: 0,
    });
    log::info!("自主移动开始：{graph}");
    Some(MotionEvent::Move { id, graph })
}

/// Move 的 start 段播完后才定位并启动位移计时器，和原版 Move.Display 一致。
#[tauri::command]
pub fn begin_pet_motion_step(
    app: AppHandle,
    state: tauri::State<'_, MotionState>,
    id: u64,
) -> bool {
    let snapshot = state
        .lock()
        .ok()
        .and_then(|motion| motion.active.clone())
        .filter(|active| active.id == id);
    let Some(active) = snapshot else {
        return false;
    };
    let Some(win) = app.get_webview_window(PET_WINDOW) else {
        return false;
    };
    let Some(g) = geometry_in(&win, active.bounds) else {
        return false;
    };
    let rule = state.lock().ok().and_then(|motion| {
        motion
            .profile
            .as_ref()?
            .moves
            .get(active.rule_index)
            .cloned()
    });
    let Some(rule) = rule else {
        return false;
    };
    let (x, y) = located_position(g, rule.locate_type, rule.locate_length);
    if (x != g.x || y != g.y) && win.set_position(PhysicalPosition::new(x, y)).is_err() {
        return false;
    }
    let Ok(mut motion) = state.lock() else {
        return false;
    };
    let Some(current) = motion.active.as_mut().filter(|current| current.id == id) else {
        return false;
    };
    current.stepping = true;
    current.next_step = Instant::now() + step_interval(&rule);
    true
}

/// Body 每播完一轮 loop 都问一次。原版至少走 Distance 轮，之后越来越容易停止；
/// 停止点有 40% 概率换成方向兼容且此刻可触发的另一条规则。
#[tauri::command]
pub fn complete_pet_motion_cycle(
    app: AppHandle,
    state: tauri::State<'_, MotionState>,
    id: u64,
) -> Option<MotionEvent> {
    let snapshot = state
        .lock()
        .ok()
        .and_then(|motion| motion.active.clone())
        .filter(|active| active.id == id)?;
    let win = app.get_webview_window(PET_WINDOW)?;
    let geometry = geometry_in(&win, snapshot.bounds)?;
    let decision = {
        let mut motion = state.lock().ok()?;
        decide_cycle(&mut motion, id, geometry)?
    };
    if matches!(decision, MotionEvent::MoveStop { .. }) {
        reposition_after_motion(&win, geometry);
    }
    Some(decision)
}

/// 用户触摸、说话或状态切换优先于自主移动；立即停窗口，不强迫播放 end。
#[tauri::command]
pub fn stop_pet_motion(app: AppHandle, state: tauri::State<'_, MotionState>) -> bool {
    let active = state
        .lock()
        .ok()
        .and_then(|mut motion| motion.active.take());
    let Some(active) = active else {
        return false;
    };
    log::info!("自主移动被交互中断");
    if let Some(win) = app.get_webview_window(PET_WINDOW) {
        if let Some(g) = geometry_in(&win, active.bounds) {
            reposition_after_motion(&win, g);
        }
    }
    true
}

/// 50 ms 的原生轮询上推进位置。即使 WebView 在后台被节流，窗口也不会越走越远；
/// 碰到 CheckType 边界时直接尝试原版的兼容移动切换。
pub fn poll(app: &AppHandle, win: &WebviewWindow, primary_button_held: bool) {
    if primary_button_held {
        return;
    }
    let Some(state) = app.try_state::<MotionState>() else {
        return;
    };
    let snapshot = state.lock().ok().and_then(|motion| motion.active.clone());
    let Some(active) = snapshot.filter(|active| active.stepping) else {
        return;
    };
    let now = Instant::now();
    if now < active.next_step {
        return;
    }
    let Some(g) = geometry_in(win, active.bounds) else {
        return;
    };
    let rule = state.lock().ok().and_then(|motion| {
        motion
            .profile
            .as_ref()?
            .moves
            .get(active.rule_index)
            .cloned()
    });
    let Some(rule) = rule else {
        return;
    };

    if !checked(&rule, g) {
        let decision = {
            let Ok(mut motion) = state.lock() else {
                return;
            };
            decide_boundary(&mut motion, active.id, g)
        };
        if let Some(event) = decision {
            if matches!(event, MotionEvent::MoveStop { .. }) {
                reposition_after_motion(win, g);
            }
            let _ = app.emit("pet:motion", event);
        }
        return;
    }

    let interval = step_interval(&rule);
    let late = now.saturating_duration_since(active.next_step);
    let ticks = (1 + (late.as_millis() / interval.as_millis().max(1)) as u32).min(4);
    // 先在锁内认领这一拍；stop/switch 已经换掉 id 时，旧轮询不能再动窗口。
    {
        let Ok(mut motion) = state.lock() else {
            return;
        };
        let Some(current) = motion
            .active
            .as_mut()
            .filter(|current| current.id == active.id && current.stepping)
        else {
            return;
        };
        if current.next_step != active.next_step {
            return;
        }
        current.next_step = active.next_step + interval.saturating_mul(ticks);
    }
    let scale = g.scale();
    let x = (g.x as f64 + rule.speed_x * scale * ticks as f64).round() as i32;
    let y = (g.y as f64 + rule.speed_y * scale * ticks as f64).round() as i32;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

/// 提起结束后按原版的 50 逻辑像素阈值检查左右越界，进入侧挂。
pub fn settle_after_drag(app: &AppHandle) -> bool {
    let Some(win) = app.get_webview_window(PET_WINDOW) else {
        return false;
    };
    let Some(geometry) = geometry(app, &win) else {
        return false;
    };
    let Some(state) = app.try_state::<MotionState>() else {
        return false;
    };
    let anchors = state.lock().ok().and_then(|mut motion| {
        motion.active = None;
        motion.profile.as_ref().map(|profile| profile.side)
    });
    let Some(anchors) = anchors else {
        return false;
    };
    let Some(side) = detect_side(geometry) else {
        return false;
    };
    let x = side_x(geometry, anchors, side);
    let y = geometry.clamped_y();
    if win.set_position(PhysicalPosition::new(x, y)).is_err() {
        return false;
    }
    if let Ok(mut motion) = state.lock() {
        motion.side = Some(side);
    }
    log::info!(
        "宠物侧挂到{}边",
        if side == Side::Left { "左" } else { "右" }
    );
    let _ = app.emit("pet:motion", MotionEvent::Side { side });
    true
}

/// 松手时没到侧挂的份上，但身体有一部分出了屏幕：弹回来。上方的气泡区可以在屏幕外，
/// 身体（下面的正方形）不行——拖出去了找不回来是真的会发生的事
pub fn clamp_into_screen(app: &AppHandle) -> bool {
    let Some(win) = app.get_webview_window(PET_WINDOW) else {
        return false;
    };
    let Some(g) = geometry(app, &win) else {
        return false;
    };
    let (x, y) = clamped_position(g);
    if x == g.x && y == g.y {
        return false;
    }
    log::info!("宠物出了屏幕，弹回来（{},{} → {},{}）", g.x, g.y, x, y);
    win.set_position(PhysicalPosition::new(x, y)).is_ok()
}

/// 身体完全落在当前显示器里的最近位置
fn clamped_position(g: Geometry) -> (i32, i32) {
    let max_x = (g.monitor_right() - g.width as f64).round() as i32;
    let x = if max_x >= g.monitor_x { g.x.clamp(g.monitor_x, max_x) } else { g.monitor_x };
    (x, g.clamped_y())
}

/// 点击 / 再次拖动侧挂的宠物时，把完整身体拉回当前显示器。
#[tauri::command]
pub fn exit_pet_side(app: AppHandle) -> bool {
    let Some(state) = app.try_state::<MotionState>() else {
        return false;
    };
    let side = state.lock().ok().and_then(|motion| motion.side);
    let Some(side) = side else {
        return false;
    };
    let Some(win) = app.get_webview_window(PET_WINDOW) else {
        return false;
    };
    let Some(g) = geometry(&app, &win) else {
        return false;
    };
    let x = match side {
        Side::Left => g.monitor_x,
        Side::Right => (g.monitor_right() - g.width as f64).round() as i32,
    };
    if win
        .set_position(PhysicalPosition::new(x, g.clamped_y()))
        .is_err()
    {
        return false;
    }
    if let Ok(mut motion) = state.lock() {
        motion.side = None;
    }
    let _ = app.emit("pet:motion", MotionEvent::SideStop);
    true
}

fn decide_cycle(motion: &mut PetMotion, id: u64, geometry: Geometry) -> Option<MotionEvent> {
    let active = motion.active.clone().filter(|active| active.id == id)?;
    let rules = motion.profile.as_ref()?.moves.clone();
    let rule = rules.get(active.rule_index)?;
    if !checked(rule, geometry) {
        return switch_or_stop(motion, &active, &rules, geometry);
    }

    let draw = random_below(active.walk_length);
    if let Some(current) = motion.active.as_mut().filter(|current| current.id == id) {
        current.walk_length = current.walk_length.saturating_add(1);
    }
    if draw < rule.distance {
        return Some(MotionEvent::MoveContinue { id });
    }
    switch_or_stop(motion, &active, &rules, geometry)
}

fn decide_boundary(motion: &mut PetMotion, id: u64, geometry: Geometry) -> Option<MotionEvent> {
    let active = motion.active.clone().filter(|active| active.id == id)?;
    let rules = motion.profile.as_ref()?.moves.clone();
    switch_or_stop(motion, &active, &rules, geometry)
}

fn switch_or_stop(
    motion: &mut PetMotion,
    active: &ActiveMove,
    rules: &[MoveRule],
    geometry: Geometry,
) -> Option<MotionEvent> {
    let current = rules.get(active.rule_index)?;
    if two_in_five() {
        let candidates = compatible_indices(current, rules, geometry, active.mood_bit);
        if let Some(rule_index) = random_index(candidates.len()).map(|pick| candidates[pick]) {
            let graph = rules.get(rule_index)?.graph.clone();
            let id = next_id(motion);
            motion.active = Some(ActiveMove {
                id,
                rule_index,
                mood_bit: active.mood_bit,
                bounds: active.bounds,
                stepping: false,
                next_step: Instant::now(),
                walk_length: 0,
            });
            log::info!("自主移动衔接：{} -> {graph}", current.graph);
            return Some(MotionEvent::Move { id, graph });
        }
    }
    motion.active = None;
    log::info!("自主移动结束：{}", current.graph);
    Some(MotionEvent::MoveStop {
        id: active.id,
        graph: current.graph.clone(),
    })
}

fn compatible_indices(
    current: &MoveRule,
    rules: &[MoveRule],
    geometry: Geometry,
    mood_bit: u8,
) -> Vec<usize> {
    let x_positive = current.speed_x > 0.0;
    let y_positive = current.speed_y > 0.0;
    rules
        .iter()
        .enumerate()
        .filter_map(|(i, candidate)| {
            let mut score = 0;
            if current.speed_x != 0.0 && candidate.speed_x != 0.0 {
                score += if (candidate.speed_x > 0.0) == x_positive {
                    1
                } else {
                    -1
                };
            }
            if current.speed_y != 0.0 && candidate.speed_y != 0.0 {
                score += if (candidate.speed_y > 0.0) == y_positive {
                    1
                } else {
                    -1
                };
            }
            (score >= 0 && triggered(candidate, geometry, mood_bit)).then_some(i)
        })
        .collect()
}

fn triggered(rule: &MoveRule, geometry: Geometry, mood_bit: u8) -> bool {
    rule.mode_type & mood_bit != 0
        && directions_match(
            rule.trigger_type,
            geometry,
            Limits {
                left: rule.trigger_left,
                right: rule.trigger_right,
                top: rule.trigger_top,
                bottom: rule.trigger_bottom,
            },
        )
}

fn checked(rule: &MoveRule, geometry: Geometry) -> bool {
    directions_match(
        rule.check_type,
        geometry,
        Limits {
            left: rule.check_left,
            right: rule.check_right,
            top: rule.check_top,
            bottom: rule.check_bottom,
        },
    )
}

fn directions_match(flags: u16, geometry: Geometry, limits: Limits) -> bool {
    if flags == 0 {
        return true;
    }
    let d = geometry.distances();
    let scale = geometry.scale();
    !((flags & LEFT != 0 && d.left > limits.left * scale)
        || (flags & RIGHT != 0 && d.right > limits.right * scale)
        || (flags & TOP != 0 && d.top > limits.top * scale)
        || (flags & BOTTOM != 0 && d.bottom > limits.bottom * scale)
        || (flags & LEFT_GREATER != 0 && d.left < limits.left * scale)
        || (flags & RIGHT_GREATER != 0 && d.right < limits.right * scale)
        || (flags & TOP_GREATER != 0 && d.top < limits.top * scale)
        || (flags & BOTTOM_GREATER != 0 && d.bottom < limits.bottom * scale))
}

fn mood_bit(mood: &str) -> Option<u8> {
    match mood {
        "happy" => Some(2),
        "nomal" => Some(4),
        "poorcondition" => Some(8),
        "ill" => Some(16),
        _ => None,
    }
}

fn step_interval(rule: &MoveRule) -> Duration {
    Duration::from_millis(rule.interval.clamp(16, 10_000))
}

fn next_id(motion: &mut PetMotion) -> u64 {
    motion.next_id = motion.next_id.wrapping_add(1).max(1);
    motion.next_id
}

fn random_index(len: usize) -> Option<usize> {
    (len > 0).then(|| ((roll() * len as f32) as usize).min(len - 1))
}

fn random_below(upper: u32) -> u32 {
    if upper == 0 {
        0
    } else {
        (roll() * upper as f32) as u32
    }
}

fn two_in_five() -> bool {
    random_below(5) <= 1
}

fn geometry(app: &AppHandle, win: &WebviewWindow) -> Option<Geometry> {
    let pos = win.outer_position().ok()?;
    let size = win.inner_size().ok()?;
    let monitor = win
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())?;
    Some(Geometry {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        monitor_x: monitor.position().x,
        monitor_y: monitor.position().y,
        monitor_width: monitor.size().width,
        monitor_height: monitor.size().height,
    })
}

fn geometry_in(win: &WebviewWindow, bounds: ScreenBounds) -> Option<Geometry> {
    let pos = win.outer_position().ok()?;
    let size = win.inner_size().ok()?;
    Some(Geometry {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        monitor_x: bounds.x,
        monitor_y: bounds.y,
        monitor_width: bounds.width,
        monitor_height: bounds.height,
    })
}

fn located_position(g: Geometry, locate: LocateType, length: f64) -> (i32, i32) {
    let amount = length * g.scale();
    match locate {
        LocateType::None => (g.x, g.y),
        LocateType::Left => ((g.monitor_x as f64 - amount).round() as i32, g.y),
        LocateType::Right => (
            (g.monitor_right() - g.width as f64 + amount).round() as i32,
            g.y,
        ),
        LocateType::Top => (
            g.x,
            (g.monitor_y as f64 - g.headroom() as f64 - amount).round() as i32,
        ),
        LocateType::Bottom => (
            g.x,
            (g.monitor_bottom() - g.height as f64 + amount).round() as i32,
        ),
    }
}

fn repositioned(g: Geometry) -> (i32, i32) {
    let d = g.distances();
    let threshold = -(g.width as f64) * 0.25;
    let x = if d.left < threshold {
        g.monitor_x
    } else if d.right < threshold {
        (g.monitor_right() - g.width as f64).round() as i32
    } else {
        g.x
    };
    let y = if d.top < threshold {
        g.monitor_y - g.headroom()
    } else if d.bottom < threshold {
        (g.monitor_bottom() - g.height as f64).round() as i32
    } else {
        g.y
    };
    (x, y)
}

fn reposition_after_motion(win: &WebviewWindow, g: Geometry) {
    let (x, y) = repositioned(g);
    if x != g.x || y != g.y {
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
}

fn detect_side(g: Geometry) -> Option<Side> {
    let threshold = SIDE_TRIGGER * g.scale();
    let beyond_left = g.monitor_x as f64 - g.x as f64;
    let beyond_right = g.x as f64 + g.width as f64 - g.monitor_right();
    if beyond_left > threshold {
        Some(Side::Left)
    } else if beyond_right > threshold {
        Some(Side::Right)
    } else {
        None
    }
}

fn side_x(g: Geometry, anchors: SideAnchors, side: Side) -> i32 {
    let scale = g.scale();
    match side {
        Side::Left => (g.monitor_x as f64 - anchors.left * scale).round() as i32,
        Side::Right => (g.monitor_right() - anchors.right * scale).round() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry_at(x: i32, y: i32, width: u32, height: u32) -> Geometry {
        Geometry {
            x,
            y,
            width,
            height,
            monitor_x: 0,
            monitor_y: 0,
            monitor_width: 1920,
            monitor_height: 1080,
        }
    }

    fn rule() -> MoveRule {
        MoveRule {
            graph: "walk.left".into(),
            locate_type: LocateType::None,
            locate_length: 0.0,
            interval: 125,
            check_type: LEFT_GREATER,
            check_left: 100.0,
            check_right: 100.0,
            check_top: 100.0,
            check_bottom: 100.0,
            mode_type: 14,
            trigger_type: LEFT_GREATER,
            trigger_left: 200.0,
            trigger_right: 100.0,
            trigger_top: 100.0,
            trigger_bottom: 100.0,
            speed_x: -14.0,
            speed_y: 0.0,
            distance: 7,
        }
    }

    #[test]
    fn 松手后出屏的身体会弹回屏幕内() {
        // 屏幕 1920×1080 在 (0,0)，窗口 500 宽 800 高（上面 300 是气泡区）
        let g = geometry_at(-120, 900, 500, 800);
        assert_eq!(clamped_position(g), (0, 1080 - 800), "左边和底部都出去了");
        let g = geometry_at(1700, -400, 500, 800);
        assert_eq!(clamped_position(g), (1920 - 500, -300), "右边出去；顶部只允许气泡区出屏");
        let g = geometry_at(700, 200, 500, 800);
        assert_eq!(clamped_position(g), (700, 200), "本来就在屏幕里的不动");
    }

    #[test]
    fn 侧挂阈值严格超过五十逻辑像素() {
        assert_eq!(detect_side(geometry_at(-49, 0, 500, 800)), None);
        assert_eq!(detect_side(geometry_at(-51, 0, 500, 800)), Some(Side::Left));
        assert_eq!(
            detect_side(geometry_at(1471, 0, 500, 800)),
            Some(Side::Right)
        );

        // 300px 身体时阈值随显示尺寸缩到 30px。
        assert_eq!(detect_side(geometry_at(-29, 0, 300, 480)), None);
        assert_eq!(detect_side(geometry_at(-31, 0, 300, 480)), Some(Side::Left));
    }

    #[test]
    fn 侧挂锚点按身体尺寸缩放且支持负坐标屏幕() {
        let anchors = SideAnchors {
            left: 219.0,
            right: 281.0,
        };
        let normal = geometry_at(-51, 100, 500, 800);
        assert_eq!(side_x(normal, anchors, Side::Left), -219);
        assert_eq!(side_x(normal, anchors, Side::Right), 1639);

        let mut negative = geometry_at(-1951, 100, 300, 480);
        negative.monitor_x = -1920;
        negative.monitor_width = 1920;
        assert_eq!(detect_side(negative), Some(Side::Left));
        assert_eq!(side_x(negative, anchors, Side::Left), -2051);
    }

    #[test]
    fn 纵向夹紧只看底部身体画布() {
        assert_eq!(geometry_at(0, -900, 500, 800).clamped_y(), -300);
        assert_eq!(geometry_at(0, 900, 500, 800).clamped_y(), 280);
        assert_eq!(geometry_at(0, -900, 300, 480).clamped_y(), -180);
        assert_eq!(geometry_at(0, 900, 300, 480).clamped_y(), 600);
    }

    #[test]
    fn 原版方向位区分靠近和远离边缘() {
        let mut walk = rule();
        let far = geometry_at(250, 0, 500, 800);
        let near = geometry_at(90, 0, 500, 800);
        assert!(triggered(&walk, far, 4));
        assert!(!triggered(&walk, near, 4));
        assert!(checked(&walk, far));
        assert!(!checked(&walk, near));
        assert!(!triggered(&walk, far, 16)); // 原资源移动规则没有 Ill 位

        walk.trigger_type = LEFT;
        walk.trigger_left = 100.0;
        assert!(!triggered(&walk, far, 4));
        assert!(triggered(&walk, near, 4));
    }

    #[test]
    fn 定位公式排除头顶气泡区() {
        let g = geometry_at(600, 200, 500, 800);
        assert_eq!(located_position(g, LocateType::Left, 145.0), (-145, 200));
        assert_eq!(located_position(g, LocateType::Right, 185.0), (1605, 200));
        assert_eq!(located_position(g, LocateType::Top, 150.0), (600, -450));
        assert_eq!(located_position(g, LocateType::Bottom, 0.0), (600, 280));
    }

    #[test]
    fn 移动结束只把超过四分之一的身体拉回屏幕() {
        assert_eq!(repositioned(geometry_at(-145, 100, 500, 800)), (0, 100));
        assert_eq!(repositioned(geometry_at(-120, 100, 500, 800)), (-120, 100));
        assert_eq!(repositioned(geometry_at(100, -450, 500, 800)), (100, -300));
    }

    #[test]
    fn 兼容移动不允许直接反向但允许转上墙() {
        let current = rule();
        let mut reverse = rule();
        reverse.graph = "walk.right".into();
        reverse.speed_x = 14.0;
        reverse.trigger_type = 0;
        let mut climb = rule();
        climb.graph = "climb.left".into();
        climb.speed_x = 0.0;
        climb.speed_y = -10.0;
        climb.trigger_type = LEFT | TOP_GREATER;
        climb.trigger_left = 100.0;
        climb.trigger_top = 200.0;
        let at_left_wall = geometry_at(80, 100, 500, 800);
        let found = compatible_indices(&current, &[reverse, climb], at_left_wall, 4);
        assert_eq!(found, vec![1]);
    }
}
