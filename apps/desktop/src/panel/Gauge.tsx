/** 低于这个比例就变红，提示她快撑不住了 */
const LOW = 0.25

export const GREEN = '#7fae79'
export const YELLOW = '#d1aa62'
export const RED = '#bf7070'

/**
 * `accent` 用来给「低了也不算告急」的量换个配色——好感度低只是生分，
 * 不是撑不住了，不该和饱腹见底一个颜色。`note` 挂在数字后面（药效还剩多少）
 */
export function Gauge({ label, value, accent, note }: { label: string; value: number; accent?: string; note?: string }) {
  const pct = Math.max(0, Math.min(100, value))
  const color = accent ?? (pct / 100 < LOW ? RED : GREEN)
  // 备注另起一行，不挤进条子那一行——否则这一根就比别的短
  return (
    <div className="status-gauge">
      <div className="status-gauge-row">
        <span className="status-gauge-label">{label}</span>
        <div className="status-gauge-track">
          <div className="status-gauge-fill" style={{ width: `${pct}%`, background: color }} />
        </div>
        <span className="status-gauge-value">
          {pct.toFixed(0)}
        </span>
      </div>
      {note && <div className="status-gauge-note">{note}</div>}
    </div>
  )
}
