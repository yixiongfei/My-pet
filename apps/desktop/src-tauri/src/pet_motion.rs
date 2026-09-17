//! 宠物窗口的自主移动与贴边几何。
//!
//! 动画仍由 Body 播，Core 只负责窗口位置和模式事件。所有原版坐标都以底部
//! 500×500 的身体画布为基准；窗口上方的气泡区不参与距离和锚点计算。

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

use crate::PET_WINDOW;

const PET_LOGICAL_SIZE: f64 = 500.0;
const SIDE_TRIGGER: f64 = 50.0;

#[derive(Clone, Copy, Debug, Deserialize)]
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
#[allow(dead_code)]
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
    pub distance: f64,
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

#[derive(Default)]
pub struct PetMotion {
    profile: Option<MotionProfile>,
    side: Option<Side>,
}

pub type MotionState = Mutex<PetMotion>;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum MotionEvent {
    Side { side: Side },
    Stop,
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

impl Geometry {
    fn scale(self) -> f64 {
        self.width.max(1) as f64 / PET_LOGICAL_SIZE
    }

    fn monitor_right(self) -> f64 {
        self.monitor_x as f64 + self.monitor_width as f64
    }

    fn clamped_y(self) -> i32 {
        // 身体顶 = window y + (height - width)，身体底 = window y + height。
        let headroom = self.height.saturating_sub(self.width) as i32;
        let min = self.monitor_y - headroom;
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
    }
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
    let anchors = state
        .lock()
        .ok()
        .and_then(|motion| motion.profile.as_ref().map(|profile| profile.side));
    let Some(anchors) = anchors else {
        return false;
    };
    let side = detect_side(geometry);
    let Some(side) = side else {
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
    let _ = app.emit("pet:motion", MotionEvent::Stop);
    true
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
}
