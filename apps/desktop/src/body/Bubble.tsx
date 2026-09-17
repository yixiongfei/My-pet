/** 最多显示几行；再多就在末尾打省略号。真正能放几行还要看头顶那块地方有多高 */
const MAX_LINES = 7
/** 正文行高（px），和下面 font 的 1.55 倍行距对应 */
const LINE_PX = 14 * 1.55
/** 名字行 + 上下内边距 + 尾巴，大约这么高 */
const CHROME_PX = 52
/** 流式输出时正文超过这么多字就只显示尾巴——新字要看得见 */
const STREAM_TAIL_CHARS = 120

export interface BubbleProps {
  name: string
  text: string
  /** 还在流式输出中：显示光标，看尾巴不看头 */
  streaming: boolean
  /** 头顶区的高度（px）：气泡不能比它高，否则名字会被顶出窗口 */
  maxHeight?: number
}

/**
 * 说话气泡。浮在立绘头顶（窗口上方那块 HEAD_ROOM 区域的底部），尾巴指向头。
 * 这块区域对鼠标是穿透的，所以气泡没有任何按钮：超出的部分打省略号，
 * 完整内容在对话窗口里。
 */
export function Bubble({ name, text, streaming, maxHeight }: BubbleProps) {
  const long = text.length > STREAM_TAIL_CHARS
  const shown = streaming && long ? '…' + text.slice(-STREAM_TAIL_CHARS) : text
  const fit = maxHeight ? Math.floor((maxHeight - CHROME_PX) / LINE_PX) : MAX_LINES
  const lines = Math.max(2, Math.min(MAX_LINES, fit))
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', width: '100%' }}>
      <div
        style={{
          maxWidth: '94%', minWidth: 120, boxSizing: 'border-box',
          padding: '9px 13px 10px', borderRadius: 16,
          background: 'rgba(28,28,32,.9)', color: '#f4f4f5',
          font: `${long ? 13 : 14}px/1.55 system-ui, "Microsoft YaHei", sans-serif`,
          boxShadow: '0 6px 24px rgba(0,0,0,.35)', backdropFilter: 'blur(6px)',
        }}
      >
        <div style={{ fontWeight: 700, color: '#ffd9a0', fontSize: 12, marginBottom: 2, letterSpacing: '.2px' }}>{name}</div>
        <div
          style={{
            display: '-webkit-box', WebkitBoxOrient: 'vertical', WebkitLineClamp: lines,
            overflow: 'hidden', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
          }}
        >
          {shown}
          {streaming && <span style={{ opacity: 0.55 }}>▍</span>}
        </div>
      </div>
      {/* 指向头顶的小尾巴 */}
      <div
        style={{
          width: 14, height: 14, marginTop: -7, transform: 'rotate(45deg)',
          background: 'rgba(28,28,32,.9)', borderRadius: 2,
        }}
      />
    </div>
  )
}
