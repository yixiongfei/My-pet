import { useEffect, useRef, useState } from 'react'

/** 最多显示几行；再多就在末尾打省略号。真正能放几行还要看头顶那块地方有多高 */
const MAX_LINES = 7
/** 名字行 + 上下内边距 + 尾巴，大约这么高 */
const CHROME_PX = 52
/** 流式输出时正文超过这么多字就只显示尾巴——新字要看得见 */
const STREAM_TAIL_CHARS = 120
/** 打字机：缩短显示间隔，避免模型流已经到达但气泡还在慢慢追赶 */
const TYPE_MS = 14
const TYPE_FAST_MS = 6
const TYPE_FAST_STEP = 5
const TYPE_CATCH_UP_CHARS = 24
/** 白天 / 黑夜的钟点：白天白气泡、黑夜黑气泡（和作息一致：8 点起、19 点天黑） */
const DAY_FROM_HOUR = 7
const DAY_UNTIL_HOUR = 19

export type BubbleTheme = 'light' | 'dark'

/** 现在是白天还是黑夜。知识库 App 的主题是它自己 localStorage 里的手动开关，读不到，按时间来 */
export function themeNow(d = new Date()): BubbleTheme {
  const h = d.getHours()
  return h >= DAY_FROM_HOUR && h < DAY_UNTIL_HOUR ? 'light' : 'dark'
}

/** 字号随长度：一句短话大一点，长篇小一点 */
function fontPx(chars: number): number {
  if (chars <= 14) return 17
  if (chars <= 40) return 15
  if (chars <= 90) return 14
  return 13
}

export interface BubbleProps {
  name: string
  text: string
  /** 还在流式输出中：显示光标，看尾巴不看头 */
  streaming: boolean
  /** 头顶区的高度（px）：气泡不能比它高，否则名字会被顶出窗口 */
  maxHeight?: number
  /** `self` = 自言自语：小一号、淡一点、不带名字——这话不是对你说的 */
  tone?: 'talk' | 'self'
  theme?: BubbleTheme
}

/**
 * 说话气泡。浮在立绘头顶（窗口上方那块 HEAD_ROOM 区域的底部），尾巴指向头。
 * 这块区域对鼠标是穿透的，所以气泡没有任何按钮：超出的部分打省略号，
 * 完整内容在对话窗口里。字一个一个冒出来（打字机），文本变了接着冒，不从头来。
 */
export function Bubble({ name, text, streaming, maxHeight, tone = 'talk', theme = 'dark' }: BubbleProps) {
  const quiet = tone === 'self'
  const light = theme === 'light'
  const [shownChars, setShownChars] = useState(0)
  const textRef = useRef(text)

  // 打字机：目标文本换了一句（不是在原来基础上变长）就从头打；变长了就接着打
  useEffect(() => {
    const prev = textRef.current
    textRef.current = text
    if (!text.startsWith(prev)) setShownChars(0)
  }, [text])

  useEffect(() => {
    if (shownChars >= text.length) return
    const behind = text.length - shownChars
    const fast = behind > TYPE_CATCH_UP_CHARS
    const step = fast ? TYPE_FAST_STEP : 1
    const t = window.setTimeout(() => setShownChars((n) => Math.min(text.length, n + step)), fast ? TYPE_FAST_MS : TYPE_MS)
    return () => window.clearTimeout(t)
  }, [text, shownChars])

  const typed = text.slice(0, shownChars)
  const typing = shownChars < text.length
  const long = typed.length > STREAM_TAIL_CHARS
  const shown = streaming && long ? '…' + typed.slice(-STREAM_TAIL_CHARS) : typed
  const px = quiet ? Math.min(13, fontPx(text.length)) : fontPx(text.length)
  const linePx = px * 1.55
  const fit = maxHeight ? Math.floor((maxHeight - CHROME_PX) / linePx) : MAX_LINES
  const lines = Math.max(2, Math.min(MAX_LINES, fit))

  const bg = light ? (quiet ? 'rgba(255,255,255,.82)' : 'rgba(255,255,255,.95)') : quiet ? 'rgba(28,28,32,.72)' : 'rgba(28,28,32,.9)'
  const fg = light ? (quiet ? '#5a5a60' : '#26262b') : quiet ? '#d6d6da' : '#f4f4f5'
  const nameColor = light ? '#b8791f' : '#ffd9a0'
  const shadow = light ? '0 6px 24px rgba(0,0,0,.18)' : '0 6px 24px rgba(0,0,0,.35)'
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', width: '100%' }}>
      <div
        style={{
          maxWidth: quiet ? '80%' : '94%', minWidth: quiet ? 80 : 120, boxSizing: 'border-box',
          padding: quiet ? '6px 11px 7px' : '9px 13px 10px', borderRadius: 16,
          background: bg, color: fg,
          font: `${px}px/1.55 system-ui, "Microsoft YaHei", sans-serif`,
          fontStyle: quiet ? 'italic' : 'normal',
          boxShadow: shadow, backdropFilter: 'blur(6px)',
          border: light ? '1px solid rgba(0,0,0,.06)' : 'none',
        }}
      >
        {!quiet && <div style={{ fontWeight: 700, color: nameColor, fontSize: 12, marginBottom: 2, letterSpacing: '.2px' }}>{name}</div>}
        <div
          style={{
            display: '-webkit-box', WebkitBoxOrient: 'vertical', WebkitLineClamp: lines,
            overflow: 'hidden', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
          }}
        >
          {shown}
          {(streaming || typing) && <span style={{ opacity: 0.55 }}>▍</span>}
        </div>
      </div>
      {/* 指向头顶的小尾巴 */}
      <div
        style={{
          width: 14, height: 14, marginTop: -7, transform: 'rotate(45deg)',
          background: bg, borderRadius: 2,
          borderRight: light ? '1px solid rgba(0,0,0,.06)' : 'none',
          borderBottom: light ? '1px solid rgba(0,0,0,.06)' : 'none',
        }}
      />
    </div>
  )
}
