//! 对话窗口贴边自动隐藏。
//!
//! 把对话窗口拖到屏幕左右边缘，它就吸附过去；鼠标离开后缩成一条窄边，
//! 鼠标碰到那条边再滑出来——像 IDE 的自动隐藏侧栏。这样它可以一直开着，
//! 又不占地方。有焦点（正在打字）时不收，拖离边缘就解除吸附。
//!
//! 跟着 lib.rs 的光标轮询线程走（每 16 ms 一次），不另起线程：
//! 需要的三样东西（光标位置、窗口位置、鼠标键状态）那条线程本来就在读。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, PhysicalPosition};

/// 离屏幕边缘这么近（物理像素）就吸附
const SNAP_PX: i32 = 48;
/// 收起后露出多宽的一条，鼠标碰它就展开。Win11 的窗口外框含约 8 px 的隐形边，所以真正看得见的是一半
const PEEK_PX: i32 = 18;
/// 鼠标离开多久才收：立刻收会让人觉得它在躲你
const COLLAPSE_AFTER: Duration = Duration::from_millis(400);
/// 每一拍朝目标挪剩余距离的这么多：八九拍到位，看起来是滑出来的
const EASE: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Default)]
pub struct Dock {
    pub side: Option<Side>,
    collapsed: bool,
    /// 上一拍看到的位置。变了而且不是我们挪的 = 用户在拖
    seen: Option<(i32, i32)>,
    /// 我们自己最后一次把它放到的 x；用来区分「我们挪的」和「用户拖的」
    placed_x: Option<i32>,
    outside_since: Option<Instant>,
    target_x: Option<i32>,
    /// 用户正在拖：松手之前不评判贴不贴边
    dragging: bool,
    /// 记住吸附时的 y，窗口关了再开还回到原处
    pub last_y: Option<i32>,
}

pub type DockState = Mutex<Dock>;

/// 光标轮询线程每拍调一次
pub fn poll(app: &AppHandle, button_held: bool) {
    let Some(win) = app.get_webview_window(crate::CHAT_WINDOW) else {
        if let Some(d) = app.try_state::<DockState>() {
            if let Ok(mut d) = d.lock() {
                d.seen = None;
                d.placed_x = None;
                d.target_x = None;
                d.collapsed = false;
            }
        }
        return;
    };
    if win.is_minimized().unwrap_or(false) || !win.is_visible().unwrap_or(true) {
        return;
    }
    let (Ok(pos), Ok(size), Ok(Some(mon))) = (win.outer_position(), win.outer_size(), win.current_monitor()) else { return };
    let Some(state) = app.try_state::<DockState>() else { return };
    let Ok(mut d) = state.lock() else { return };

    let (w, h) = (size.width as i32, size.height as i32);
    let (mx, mw) = (mon.position().x, mon.size().width as i32);

    // 1. 滑动中：朝目标挪一步
    if let Some(target) = d.target_x {
        let dx = target - pos.x;
        let step = if dx.abs() <= 1 { dx } else { ((dx as f64) * EASE).round() as i32 }.clamp(-w, w);
        let step = if step == 0 { dx.signum() } else { step };
        let nx = pos.x + step;
        let _ = win.set_position(PhysicalPosition::new(nx, pos.y));
        d.placed_x = Some(nx);
        d.seen = Some((nx, pos.y));
        if nx == target {
            d.target_x = None;
        }
        return;
    }

    // 2. 用户拖了窗口（位置变了且不是我们放的）。拖的过程中不评判，松手再说
    let changed = d.seen.is_some_and(|s| s != (pos.x, pos.y));
    d.seen = Some((pos.x, pos.y));
    if changed && d.placed_x != Some(pos.x) {
        d.dragging = true;
    }
    if changed && d.side.is_some() {
        d.last_y = Some(pos.y);
    }
    if button_held {
        return;
    }
    if d.dragging || (d.side.is_none() && d.placed_x.is_none()) {
        d.dragging = false;
        let near_left = (pos.x - mx).abs() <= SNAP_PX;
        let near_right = ((mx + mw) - (pos.x + w)).abs() <= SNAP_PX;
        let side = if near_left { Some(Side::Left) } else if near_right { Some(Side::Right) } else { None };
        if side != d.side {
            log::info!("对话窗口{}", match side { Some(Side::Left) => "吸附到左边", Some(Side::Right) => "吸附到右边", None => "离开边缘" });
        }
        d.side = side;
        d.collapsed = false;
        d.outside_since = None;
        d.placed_x = Some(pos.x);
        if side.is_some() {
            d.last_y = Some(pos.y);
            d.target_x = Some(expanded_x(side.unwrap(), mx, mw, w));
        }
        return;
    }

    let Some(side) = d.side else { return };
    let expanded = expanded_x(side, mx, mw, w);
    let collapsed = match side {
        Side::Left => mx - w + PEEK_PX,
        Side::Right => mx + mw - PEEK_PX,
    };

    // 3. 吸附中：看鼠标在不在窗口 / 在不在露出的那条边上
    let Ok(cursor) = app.cursor_position() else { return };
    let (cx, cy) = (cursor.x as i32, cursor.y as i32);
    let in_y = cy >= pos.y && cy <= pos.y + h;
    let inside = in_y && cx >= pos.x - 2 && cx <= pos.x + w + 2;
    let on_strip = in_y
        && match side {
            Side::Left => cx >= mx && cx <= mx + PEEK_PX * 2,
            Side::Right => cx >= mx + mw - PEEK_PX * 2 && cx < mx + mw,
        };
    let focused = win.is_focused().unwrap_or(false);

    if d.collapsed {
        if on_strip {
            d.collapsed = false;
            d.outside_since = None;
            d.target_x = Some(expanded);
        }
        return;
    }
    if inside || focused {
        d.outside_since = None;
        return;
    }
    match d.outside_since {
        None => d.outside_since = Some(Instant::now()),
        Some(t) if t.elapsed() >= COLLAPSE_AFTER => {
            d.collapsed = true;
            d.outside_since = None;
            d.target_x = Some(collapsed);
        }
        _ => {}
    }
}

fn expanded_x(side: Side, mx: i32, mw: i32, w: i32) -> i32 {
    match side {
        Side::Left => mx,
        Side::Right => mx + mw - w,
    }
}

/// 窗口刚（重新）建好：上次是吸附着的就放回原处，展开状态。用户开窗口是想看它
pub fn place_new_window(app: &AppHandle, win: &tauri::WebviewWindow) {
    let Some(state) = app.try_state::<DockState>() else { return };
    let Ok(mut d) = state.lock() else { return };
    let (Some(side), Ok(size), Ok(Some(mon))) = (d.side, win.outer_size(), win.current_monitor()) else { return };
    let x = expanded_x(side, mon.position().x, mon.size().width as i32, size.width as i32);
    let y = d.last_y.unwrap_or(mon.position().y + 80);
    let _ = win.set_position(PhysicalPosition::new(x, y));
    d.collapsed = false;
    d.placed_x = Some(x);
    d.seen = Some((x, y));
    d.target_x = None;
    d.outside_since = None;
}
