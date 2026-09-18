/** 低于这个比例就变红，提示她快撑不住了 */
const LOW = 0.25

export const GREEN = '#7cc47f'
export const YELLOW = '#e5c07b'
export const RED = '#e06c75'

/**
 * `accent` 用来给「低了也不算告急」的量换个配色——好感度低只是生分，
 * 不是撑不住了，不该和饱腹见底一个颜色。`note` 挂在数字后面（药效还剩多少）
 */
export function Gauge({ label, value, accent, note }: { label: string; value: number; accent?: string; note?: string }) {
  const pct = Math.max(0, Math.min(100, value))
  const color = accent ?? (pct / 100 < LOW ? RED : GREEN)
  // 备注另起一行，不挤进条子那一行——否则这一根就比别的短
  return (
    <div style={{ marginBottom: 8 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span style={{ width: 40, color: '#8a8a93', fontSize: 13 }}>{label}</span>
        <div style={{ flex: 1, height: 8, borderRadius: 4, background: '#2c2c32', overflow: 'hidden' }}>
          <div style={{ width: `${pct}%`, height: '100%', background: color, transition: 'width .3s' }} />
        </div>
        <span style={{ width: 34, textAlign: 'right', fontVariantNumeric: 'tabular-nums', fontSize: 13 }}>
          {pct.toFixed(0)}
        </span>
      </div>
      {note && <div style={{ color: '#8a8a93', fontSize: 12, textAlign: 'right', marginTop: 2 }}>{note}</div>}
    </div>
  )
}
