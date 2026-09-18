//! 本机知识库（obsidian-knowledge-base）的只读接入。
//!
//! 知识库 App 是独立的 Electron 程序，HTTP 端口每次启动都变，窗口关了接口就没了；
//! 但它的**唯一真相**是 vault 下的 `.kb/index.db`（SQLite，WAL）。我们直接只读这张库：
//! 知识库没开也能提醒，而且我们永远不会改它的数据。vault 在哪由 App 自己的
//! `%APPDATA%\obsidian-knowledge-base\config.json` 的 `vaultRoot` 说了算，不用再配一遍。
//!
//! 只取两样东西：当天的日程（`events`：date / title / note / kind / done，note 里带
//! `20:00–21:00` 这样的时段）和到期该复习的笔记数（`notes.next_review <= 今天`）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

/// 知识库锁着的时候最多等这么久。等太久会拖住心跳线程
const BUSY_MS: u64 = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KbEvent {
    pub id: String,
    pub title: String,
    /// 开始 / 结束时刻，从零点起的分钟数；note 里没写时段就是 None
    pub start: Option<u32>,
    pub end: Option<u32>,
    pub kind: String,
    pub done: bool,
}

/// 某一天的安排
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agenda {
    pub date: String,
    /// 按开始时间排；没写时段的排最后
    pub events: Vec<KbEvent>,
    /// 到期该复习的笔记数
    pub reviews_due: usize,
}

impl Agenda {
    pub fn pending(&self) -> impl Iterator<Item = &KbEvent> {
        self.events.iter().filter(|e| !e.done)
    }
}

/// 知识库 App 记录的 vault → `.kb/index.db`。找不到就是「这台机器没装知识库」
pub fn locate() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let config = Path::new(&appdata).join("obsidian-knowledge-base").join("config.json");
    let text = std::fs::read_to_string(config).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let root = v.get("vaultRoot")?.as_str()?;
    let db = Path::new(root).join(".kb").join("index.db");
    db.is_file().then_some(db)
}

/// 读某一天的安排。只读打开：WAL 模式下读不阻塞知识库自己的写
pub fn agenda(db: &Path, date: &str) -> Result<Agenda, String> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("打不开知识库：{e}"))?;
    conn.busy_timeout(Duration::from_millis(BUSY_MS)).map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT id, title, note, kind, done FROM events WHERE date = ?1")
        .map_err(|e| e.to_string())?;
    let mut events: Vec<KbEvent> = stmt
        .query_map([date], |r| {
            let note: String = r.get(2)?;
            let (start, end) = parse_span(&note);
            Ok(KbEvent {
                id: r.get(0)?,
                title: r.get(1)?,
                start,
                end,
                kind: r.get(3)?,
                done: r.get::<_, i64>(4)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    events.sort_by_key(|e| e.start.unwrap_or(u32::MAX));

    let reviews_due: i64 = conn
        .query_row(
            "SELECT count(*) FROM notes WHERE reviewable = 1 AND next_review IS NOT NULL AND next_review <= ?1",
            [date],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(Agenda { date: date.into(), events, reviews_due: reviews_due.max(0) as usize })
}

/// note 里的时段：`20:00–21:00`（知识库用的是 en dash，也认 `-` `~` `—`）。
/// 返回 (开始, 结束) 的分钟数；只写了开始就只有开始
pub fn parse_span(note: &str) -> (Option<u32>, Option<u32>) {
    let chars: Vec<char> = note.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if let Some((start, next)) = clock_at(&chars, i) {
            // 后面紧跟一个分隔符再一个时刻，就是结束
            let mut j = next;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let end = if j < chars.len() && matches!(chars[j], '–' | '-' | '~' | '—' | '～') {
                let mut k = j + 1;
                while k < chars.len() && chars[k] == ' ' {
                    k += 1;
                }
                clock_at(&chars, k).map(|(e, _)| e)
            } else {
                None
            };
            return (Some(start), end);
        }
        i += 1;
    }
    (None, None)
}

/// `chars[i..]` 是不是 `H:MM` / `HH:MM`。返回分钟数和紧接着的下标
fn clock_at(chars: &[char], i: usize) -> Option<(u32, usize)> {
    let digit = |c: char| c.to_digit(10);
    // 前面紧贴着数字的不算：`25:00` 里的 `5:00` 不是五点
    if i > 0 && digit(chars[i - 1]).is_some() {
        return None;
    }
    let mut h = digit(*chars.get(i)?)?;
    let mut j = i + 1;
    if let Some(d) = chars.get(j).and_then(|c| digit(*c)) {
        h = h * 10 + d;
        j += 1;
    }
    if *chars.get(j)? != ':' {
        return None;
    }
    let m = digit(*chars.get(j + 1)?)? * 10 + digit(*chars.get(j + 2)?)?;
    // 时刻后面不能再跟数字（避免把 `p33 32:10:00` 这种当成时刻——虽然不太可能）
    if chars.get(j + 3).and_then(|c| digit(*c)).is_some() {
        return None;
    }
    (h < 24 && m < 60).then_some((h * 60 + m, j + 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 认得知识库的时段写法() {
        assert_eq!(
            parse_span("plan:BV1CAxaeHEeH:43 · 20:00–21:00 · 视频 43 分钟，按 60 分钟排 · https://x/?p=43"),
            (Some(20 * 60), Some(21 * 60))
        );
        assert_eq!(parse_span("9:30-10:15 早读"), (Some(9 * 60 + 30), Some(10 * 60 + 15)));
        assert_eq!(parse_span("22:30 开始"), (Some(22 * 60 + 30), None));
        assert_eq!(parse_span(""), (None, None));
        assert_eq!(parse_span("视频 43 分钟"), (None, None));
        assert_eq!(parse_span("25:00–26:00"), (None, None), "不是时刻");
    }

    #[test]
    fn 从临时库里读当天安排() {
        let dir = std::env::temp_dir().join(format!("vpet-kb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("index.db");
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE events (id TEXT PRIMARY KEY, date TEXT NOT NULL, title TEXT NOT NULL,
                   note TEXT NOT NULL DEFAULT '', kind TEXT NOT NULL DEFAULT 'plan', done INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE notes (id TEXT PRIMARY KEY, next_review TEXT, reviewable INTEGER NOT NULL DEFAULT 1);
                 INSERT INTO events VALUES ('b','2026-09-18','晚一点的','21:00–22:00','study',0);
                 INSERT INTO events VALUES ('a','2026-09-18','早一点的','20:00–21:00','study',0);
                 INSERT INTO events VALUES ('c','2026-09-18','做完的','','plan',1);
                 INSERT INTO events VALUES ('d','2026-09-19','明天的','10:00–11:00','study',0);
                 INSERT INTO notes VALUES ('n1','2026-09-17',1);
                 INSERT INTO notes VALUES ('n2','2026-09-18',1);
                 INSERT INTO notes VALUES ('n3','2026-09-30',1);
                 INSERT INTO notes VALUES ('n4','2026-09-01',0);",
            )
            .unwrap();
        }
        let a = agenda(&path, "2026-09-18").unwrap();
        assert_eq!(a.events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a", "b", "c"], "按开始时间排，没时段的在最后");
        assert_eq!(a.pending().count(), 2);
        assert_eq!(a.reviews_due, 2, "到期两篇；不可复习的不算");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
