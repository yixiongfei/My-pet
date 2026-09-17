import { useEffect, useState } from 'react'

export interface CountdownProps {
  label: string
  /** 起止时刻（Unix 毫秒），进度环按这两个算 */
  startedAt: number
  dueAt: number
  /** 气泡在的时候缩成右上角的一粒药丸，把头顶让给话 */
  compact?: boolean
}

const R = 25
const CIRC = 2 * Math.PI * R

const mmss = (ms: number) => {
  const s = Math.max(0, Math.round(ms / 1000))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}` : `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
}

/**
 * 头顶的倒计时：一圈渐变进度环 + 剩余时间 + 标签。番茄钟、「学习一个小时」都挂这个。
 * 每秒重算一次，进度用 CSS transition 平滑过去
 */
export function Countdown({ label, startedAt, dueAt, compact }: CountdownProps) {
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(t)
  }, [])
  const total = Math.max(1, dueAt - startedAt)
  const left = Math.max(0, dueAt - now)
  const done = 1 - left / total
  const lastMinute = left <= 60_000
  if (compact) {
    const r = 7
    const c = 2 * Math.PI * r
    return (
      <div
        title={label}
        style={{
          display: 'inline-flex', alignItems: 'center', gap: 5, padding: '3px 9px 3px 6px', borderRadius: 999,
          background: 'rgba(20,20,26,.86)', color: '#fff', font: '11px/1 system-ui, sans-serif', fontWeight: 700,
          fontVariantNumeric: 'tabular-nums', boxShadow: '0 3px 10px rgba(0,0,0,.3)',
        }}
      >
        <svg width={18} height={18} viewBox="0 0 18 18">
          <circle cx={9} cy={9} r={r} fill="none" stroke="rgba(255,255,255,.18)" strokeWidth={2.5} />
          <circle cx={9} cy={9} r={r} fill="none" stroke={lastMinute ? '#ffb27a' : '#8ab4f8'} strokeWidth={2.5} strokeLinecap="round"
            strokeDasharray={c} strokeDashoffset={c * (1 - done)} transform="rotate(-90 9 9)" style={{ transition: 'stroke-dashoffset 1s linear' }} />
        </svg>
        {mmss(left)}
      </div>
    )
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2, filter: 'drop-shadow(0 4px 12px rgba(0,0,0,.35))' }}>
      <svg width={64} height={64} viewBox="0 0 64 64" style={{ display: 'block' }}>
        <defs>
          <linearGradient id="vpet-ring" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor={lastMinute ? '#ffb27a' : '#8ab4f8'} />
            <stop offset="1" stopColor={lastMinute ? '#ff7a9c' : '#c98bdb'} />
          </linearGradient>
        </defs>
        <circle cx={32} cy={32} r={29} fill="rgba(20,20,26,.86)" />
        <circle cx={32} cy={32} r={R} fill="none" stroke="rgba(255,255,255,.14)" strokeWidth={5} />
        <circle
          cx={32} cy={32} r={R} fill="none" stroke="url(#vpet-ring)" strokeWidth={5} strokeLinecap="round"
          strokeDasharray={CIRC} strokeDashoffset={CIRC * (1 - done)}
          transform="rotate(-90 32 32)"
          style={{ transition: 'stroke-dashoffset 1s linear' }}
        />
        <text x={32} y={36} textAnchor="middle" fill="#fff" fontSize={left >= 3_600_000 ? 11 : 13} fontWeight={700} fontFamily="system-ui, sans-serif" style={{ fontVariantNumeric: 'tabular-nums' }}>
          {mmss(left)}
        </text>
      </svg>
      <div
        style={{
          maxWidth: 140, padding: '2px 9px', borderRadius: 999, background: 'rgba(20,20,26,.86)',
          color: '#e9e9ee', font: '11px/1.4 system-ui, "Microsoft YaHei", sans-serif',
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}
      >
        {label}
      </div>
    </div>
  )
}
