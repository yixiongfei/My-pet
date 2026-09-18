import { useEffect, useState } from 'react'
import type { PetState, Verdict } from '@vpet/shared'
import { invokeCore } from '../body/ipc'
import { subscribePetState } from '../body/petState'
import { Gauge, GREEN, RED, YELLOW } from './Gauge'

/** 面板刷新节奏。Core 只在换动作时推事件，数值得自己拉 */
const REFRESH_MS = 1000

interface AuditRow {
  at: number
  tool: string
  origin: string
  decision: 'allow' | 'ask' | 'deny'
  ok: boolean
  input: string
  summary: string
}

const DECISION_COLOR: Record<AuditRow['decision'], string> = {
  allow: '#728a66',
  ask: '#a67f54',
  deny: '#b66d6d',
}

interface Pomo {
  phase: 'focus' | 'shortbreak' | 'longbreak'
  remainingSec: number
  completed: number
}

const PHASE_LABEL: Record<Pomo['phase'], string> = {
  focus: '专注',
  shortbreak: '短休',
  longbreak: '长休',
}

const mmss = (sec: number) => {
  const s = Math.max(0, Math.round(sec))
  return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
}

interface MemoryRow {
  id: string
  content: string
  type: string
  importance: number
  confidence: number
  source: string
  pinned: boolean
  createdAt: number
  updatedAt: number
  lastAccessedAt: number
  accessCount: number
  expiresAt: number | null
  status: 'active' | 'archived' | 'deleted'
}

interface MemoryHealth {
  active: number
  archived: number
  deleted: number
  expired: number
  usedRatio: number
  avgImportance: number
  needsSweep: number
}

const MEM_TYPE_LABEL: Record<string, string> = {
  profile: '长期',
  preference: '偏好',
  habit: '习惯',
  temporary_context: '临时',
  relationship: '互动',
  commitment: '约定',
}

const MEM_SOURCE_LABEL: Record<string, string> = {
  user_explicit: '你说的',
  user_confirmed: '你确认过',
  inferred: '推断·未确认',
  system_event: '系统记的',
}

const MEM_STATUS_COLOR: Record<MemoryRow['status'], string> = {
  active: '#728a66',
  archived: '#929b8b',
  deleted: '#b66d6d',
}

const ymd = (ms: number) => new Date(ms).toISOString().slice(0, 10)

type EmbedState =
  | { state: 'disabled'; reason: string }
  | { state: 'loading' }
  | { state: 'ready'; name: string; dim: number }
  | { state: 'failed'; reason: string }

interface MemHit {
  item: MemoryRow
  similarity: number
  lexical: number
  dense: number | null
  score: number
}

interface BiasRow {
  tag: string
  weight: number
  halfLife: number
  age: number
}

const BIAS_LABEL: Record<string, string> = { work: '工作', study: '学习', play: '玩' }

interface TimerRow {
  id: string
  label: string
  dueAt: number
  repeatMs: number | null
}

/** 你让她做的事还剩多久（Core 的 directive） */
interface DirectiveRow {
  target: string
  left: number
}

/** 与 Rust 侧 state_machine.rs 的 SICK_HEALTH / ILL_HEALTH 成对改 */
const SICK = 50
const ILL = 25

/** 药单里的一项（Core 的 FoodItem，只用到这几个字段） */
interface Medicine {
  id: string
  name: string
  health: number
  feeling: number
  price: number
}

const TARGET_LABEL: Record<string, string> = {
  work: '工作', study: '学习', play: '玩', rest: '休息', eat: '吃饭', drink: '喝水', sleep: '睡觉',
}
const ACTIVITY_LABEL: Record<string, string> = {
  idle: '空闲', working: '工作', break: '休息', studying: '学习', sleeping: '睡觉',
  playing: '玩耍', eating: '吃饭', drinking: '喝水', gift: '收礼物',
}
const MOOD_LABEL: Record<string, string> = {
  happy: '开心', nomal: '平常', poorcondition: '状态不佳', ill: '生病',
}

/** 还剩多久。已经过点了就显示「就绪」——下一拍心跳会把它响掉 */
function remaining(dueAt: number): string {
  const left = Math.round((dueAt - Date.now()) / 1000)
  if (left <= 0) return '就绪'
  if (left < 60) return `${left}s`
  const m = Math.floor(left / 60)
  return m < 60 ? `${m}m${left % 60}s` : `${Math.floor(m / 60)}h${m % 60}m`
}

/**
 * 设置页一打开就看到的「此刻」：她在做什么、为什么、心情怎样，再带四个数。
 * 放在原来「刚刚好的陪伴距离」那块的位置——先看她，再调设置
 */
export function NowHero() {
  const [state, setState] = useState<PetState | null>(null)
  useEffect(() => {
    void invokeCore<PetState>('get_pet_state').then((s) => s && setState(s))
    return subscribePetState(setState)
  }, [])
  if (!state) return <><div className="section-kicker">此刻</div><h2>正在读取状态…</h2></>
  const what = state.action?.name ?? '闲着'
  const why = state.action?.reason ? `因为${state.action.reason}` : ''
  const food = state.action?.food ? ` · ${state.action.food.name}` : ''
  return <>
    <div className="section-kicker">此刻 · {ACTIVITY_LABEL[state.activity] ?? state.activity} · {MOOD_LABEL[state.mood] ?? state.mood}</div>
    <h2>{what}<span style={{ fontSize: 15, color: '#748b68', marginLeft: 12, fontFamily: 'system-ui, "Microsoft YaHei", sans-serif' }}>{why}{food}</span></h2>
    <p className="section-description" style={{ marginBottom: 18 }}>
      体力 {state.strength.toFixed(0)} · 心情 {state.feeling.toFixed(0)} · 饱腹 {state.hunger.toFixed(0)} · 口渴 {state.thirst.toFixed(0)} · 健康 {state.health.toFixed(0)} · 好感 {state.affection.toFixed(0)}
    </p>
  </>
}

export function Panel() {
  const [state, setState] = useState<PetState | null>(null)
  const [medicines, setMedicines] = useState<Medicine[]>([])
  const [fed, setFed] = useState<string | null>(null)
  const [timers, setTimers] = useState<TimerRow[]>([])
  const [pomo, setPomo] = useState<Pomo | null>(null)
  const [audit, setAudit] = useState<AuditRow[]>([])
  const [verdict, setVerdict] = useState<Verdict | null>(null)
  const [directive, setDirective] = useState<DirectiveRow | null>(null)
  const [biases, setBiases] = useState<BiasRow[]>([])
  const [mems, setMems] = useState<MemoryRow[]>([])
  const [memHealth, setMemHealth] = useState<MemoryHealth | null>(null)
  const [memQuery, setMemQuery] = useState('')
  const [memCtx, setMemCtx] = useState('')
  const [memHits, setMemHits] = useState<MemHit[]>([])
  const [embed, setEmbed] = useState<EmbedState | null>(null)

  useEffect(() => {
    void invokeCore<Medicine[]>('list_medicines').then((m) => m && setMedicines(m))
    const pull = () => {
      void invokeCore<PetState>('get_pet_state').then((s) => s && setState(s))
      void invokeCore<TimerRow[]>('list_timers').then((t) => t && setTimers(t))
      void invokeCore<Pomo | null>('get_pomodoro').then(setPomo)
      void invokeCore<AuditRow[]>('recent_audit', { limit: 12 }).then((a) => a && setAudit(a))
      void invokeCore<BiasRow[]>('list_biases').then((b) => b && setBiases(b))
      void invokeCore<DirectiveRow | null>('get_directive').then(setDirective)
      void invokeCore<MemoryRow[]>('list_memories').then((m) => m && setMems(m))
      void invokeCore<MemoryHealth>('memory_health').then(setMemHealth)
      void invokeCore<EmbedState>('get_embed_state').then(setEmbed)
    }
    pull()
    const timer = window.setInterval(pull, REFRESH_MS)
    const stop = subscribePetState(setState)
    return () => {
      window.clearInterval(timer)
      stop()
    }
  }, [])

  const patch = (p: Record<string, number>) => void invokeCore('debug_patch_pet_state', p)
  const feed = (id?: string) =>
    void invokeCore<string>('give_medicine', id ? { id } : {}).then((n) => setFed(n ? `喂了：${n}` : '（她说不用吃药）'))
  const pullMems = () => {
    void invokeCore<MemoryRow[]>('list_memories').then((m) => m && setMems(m))
    void invokeCore<MemoryHealth>('memory_health').then(setMemHealth)
  }
  const memStatus = (id: string, status: string) =>
    void invokeCore('set_memory_status', { id, status }).then(pullMems)
  const memPin = (id: string, pinned: boolean) =>
    void invokeCore('pin_memory', { id, pinned }).then(pullMems)
  /** 预览「这个问题会带上哪些记忆」——排序的黑盒不给人看就成了玄学 */
  const previewCtx = () => {
    void invokeCore<string>('memory_context', { query: memQuery }).then((c) => {
      setMemCtx(c ?? '')
      pullMems()
    })
    // 同时把每条的字面分 / 语义分拉出来——看得见是哪一路召回的，才调得动
    void invokeCore<MemHit[]>('search_memory', { query: memQuery }).then((h) => setMemHits(h ?? []))
  }
  const rebuildIndex = () =>
    void invokeCore<number>('rebuild_embeddings').then(() => {
      pullMems()
      void invokeCore<EmbedState>('get_embed_state').then(setEmbed)
    })
  const bias = (tag: string, weight: number) =>
    void invokeCore<BiasRow[]>('set_bias', { tag, weight }).then((b) => b && setBiases(b))
  const unbias = (tag?: string) =>
    void invokeCore<BiasRow[]>('clear_bias', { tag }).then((b) => b && setBiases(b))
  /** 使唤她一次。返回的是判定结果，不是「已执行」 */
  const askFor = (target: string, minutes?: number) =>
    void invokeCore<Verdict | null>('request_action', { target, minutes }).then(setVerdict)
  const pullTimers = () => void invokeCore<TimerRow[]>('list_timers').then((t) => t && setTimers(t))
  const addTimer = (duration: string, label: string, repeat = false) =>
    void invokeCore('create_timer', { duration, label, repeat }).then(pullTimers)
  const callTool = (name: string, input: Record<string, unknown>) =>
    void invokeCore('run_tool', {
      call: { callId: `panel-${Date.now()}`, name, input, origin: 'user' },
    }).then(() => {
      pullTimers()
      void invokeCore<AuditRow[]>('recent_audit', { limit: 12 }).then((a) => a && setAudit(a))
    })

  return (
    <div className="status-panel">
      {!state ? (
        <p className="status-empty">正在读取状态…</p>
      ) : (
        <>
          <Card title="此刻">
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' }}>
              <strong style={{ fontSize: 22 }}>{state.action?.name ?? '—'}</strong>
              {state.action?.reason && (
                <span style={{ color: '#748b68' }}>因为{state.action.reason}</span>
              )}
              {state.action?.food && (
                <span style={{ color: '#778f6b' }}>· {state.action.food.name}</span>
              )}
            </div>
            <div style={{ color: '#929b8b', fontSize: 13, marginTop: 6 }}>
              {ACTIVITY_LABEL[state.activity] ?? state.activity} · {MOOD_LABEL[state.mood] ?? state.mood}
            </div>
          </Card>

          <Card title="状态数值">
            <Gauge label="体力" value={state.strength} />
            <Gauge label="心情" value={state.feeling} />
            <Gauge label="饱腹" value={state.hunger} />
            <Gauge label="口渴" value={state.thirst} />
            <Gauge label="好感" value={state.affection} accent="#a987b7" />
            <Gauge
              label="健康"
              value={state.health}
              accent={state.health < ILL ? RED : state.health < SICK ? YELLOW : GREEN}
              note={state.remedy > 0 ? `药效还有 +${state.remedy.toFixed(0)}` : undefined}
            />
            <div style={{ display: 'flex', gap: 24, marginTop: 14, fontSize: 15 }}>
              <span>💰 {state.money.toFixed(1)}</span>
              <span>⭐ Lv{state.level}</span>
              <span style={{ color: '#929b8b' }}>经验 {state.exp.toFixed(0)}</span>
            </div>
          </Card>

          {state.health < SICK && (
            <Card title={state.health < ILL ? '她病得很重' : '她生病了'}>
              <p className="status-alert">健康 {state.health.toFixed(0)}，请选择药物。</p>
              <Row>
                <Btn onClick={() => feed()}>喂药（自动挑一种）</Btn>
                {medicines.map((m) => (
                  <Btn key={m.id} onClick={() => feed(m.id)}>
                    {m.name} +{m.health}
                    {m.feeling ? ` · 心情${m.feeling > 0 ? '+' : ''}${m.feeling}` : ''}
                  </Btn>
                ))}
              </Row>
              {fed && <p style={{ color: '#708a65', marginBottom: 0 }}>{fed}</p>}
            </Card>
          )}

          <Card title="行为控制">
            <Row>
              <Btn onClick={() => askFor('work')}>去工作</Btn>
              <Btn onClick={() => askFor('study')}>去学习</Btn>
              <Btn onClick={() => askFor('play')}>去玩</Btn>
              <Btn onClick={() => askFor('play', 10)}>玩 10 分钟</Btn>
              <Btn onClick={() => askFor('rest')}>去休息</Btn>
              <Btn onClick={() => askFor('sleep')}>去睡觉</Btn>
              <Btn onClick={() => askFor('eat')}>去吃饭</Btn>
              <Btn onClick={() => askFor('drink')}>去喝水</Btn>
            </Row>
            {verdict && (
              <p style={{ marginBottom: 0, color: verdict.obey ? '#708a65' : '#a67f54' }}>
                {verdict.obey ? '✓' : '✗'}「{verdict.action}」· {verdict.say}
                <span style={{ color: '#929b8b', marginLeft: 8, fontVariantNumeric: 'tabular-nums' }}>
                  （这次有 {(verdict.p * 100).toFixed(0)}% 会听{verdict.refusal ? ` · ${verdict.refusal}` : ''}
                  {verdict.obey && verdict.minutes ? ` · 接下来 ${verdict.minutes.toFixed(0)} 分钟` : ''}）
                </span>
              </p>
            )}
            {directive && (
              <p style={{ margin: '8px 0 0', color: '#748b68', fontSize: 13 }}>
                正在按你说的{TARGET_LABEL[directive.target] ?? directive.target}，还剩 {Math.ceil(directive.left)} 分钟
              </p>
            )}
          </Card>

          <Card title="她记得什么">
            {memHealth && (
              <p className="status-meta">
                活跃 {memHealth.active} · 归档 {memHealth.archived} · 已删 {memHealth.deleted} ·
                使用率 {(memHealth.usedRatio * 100).toFixed(0)}%
                {memHealth.needsSweep > 0 && (
                  <span> · {memHealth.needsSweep} 条待整理</span>
                )}
                {embed?.state === 'ready' && <Btn onClick={rebuildIndex}>重建检索</Btn>}
              </p>
            )}
            <div style={{ display: 'flex', gap: 8, marginBottom: 12 }}>
              <input
                className="status-input"
                value={memQuery}
                onChange={(e) => setMemQuery(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && previewCtx()}
                placeholder="试一个问题，看会带上哪些记忆"
              />
              <Btn onClick={previewCtx}>检索</Btn>
            </div>
            {memHits.length > 0 && (
              <ul style={{ listStyle: 'none', margin: '0 0 10px', padding: 0, fontSize: 12 }}>
                {memHits.map((h) => (
                  <li key={h.item.id} style={{ color: '#929b8b', marginBottom: 3 }}>
                    <span style={{ fontVariantNumeric: 'tabular-nums' }}>
                      总分 {h.score.toFixed(3)} ← 字面 {h.lexical.toFixed(2)} ·{' '}
                      {h.dense == null ? '语义 —' : `语义 ${h.dense.toFixed(2)}`}
                    </span>
                    <span style={{ color: '#596852', marginLeft: 8 }}>{h.item.content}</span>
                  </li>
                ))}
              </ul>
            )}
            {memCtx && (
              <pre
                className="status-memory-preview"
              >
                {memCtx}
              </pre>
            )}
            {mems.length === 0 ? (
              <p className="status-empty">暂无记忆</p>
            ) : (
              <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
                {mems.map((m) => (
                  <li
                    key={m.id}
                    style={{
                      display: 'flex',
                      alignItems: 'baseline',
                      gap: 8,
                      fontSize: 13,
                      marginBottom: 8,
                      opacity: m.status === 'active' ? 1 : 0.5,
                    }}
                  >
                    <span style={{ color: MEM_STATUS_COLOR[m.status], fontSize: 11 }}>●</span>
                    <span style={{ color: '#748b68', width: 36 }}>
                      {MEM_TYPE_LABEL[m.type] ?? m.type}
                    </span>
                    <span
                      style={{
                        flex: 1,
                        textDecoration: m.status === 'deleted' ? 'line-through' : 'none',
                      }}
                    >
                      {m.pinned && '📌 '}
                      {m.content}
                    </span>
                    <span style={{ color: '#929b8b', fontSize: 11, whiteSpace: 'nowrap' }}>
                      {MEM_SOURCE_LABEL[m.source] ?? m.source} · 重{m.importance.toFixed(0)} · 用
                      {m.accessCount}
                      {m.expiresAt && ` · 至${ymd(m.expiresAt)}`}
                    </span>
                    {m.status === 'active' && (
                      <>
                        <Btn onClick={() => memPin(m.id, !m.pinned)}>{m.pinned ? '取消置顶' : '置顶'}</Btn>
                        <Btn onClick={() => memStatus(m.id, 'archived')}>归档</Btn>
                        <Btn onClick={() => memStatus(m.id, 'deleted')}>删除</Btn>
                      </>
                    )}
                    {m.status === 'archived' && <Btn onClick={() => memStatus(m.id, 'active')}>恢复</Btn>}
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card title="长期倾向">
            <Row>
              <Btn onClick={() => bias('work', 1)}>多工作</Btn>
              <Btn onClick={() => bias('study', 1)}>多学习</Btn>
              <Btn onClick={() => bias('play', -1)}>少玩点</Btn>
              <Btn onClick={() => unbias()}>全撤</Btn>
            </Row>
            {biases.length === 0 ? (
              <p className="status-empty">暂无倾向</p>
            ) : (
              <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
                {biases.map((b) => (
                  <li
                    key={b.tag}
                    style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 14, marginBottom: 6 }}
                  >
                    <span style={{ width: 44 }}>{BIAS_LABEL[b.tag] ?? b.tag}</span>
                    <span
                      style={{
                        color: b.weight > 0 ? '#708a65' : '#b66d6d',
                        fontVariantNumeric: 'tabular-nums',
                        width: 52,
                      }}
                    >
                      {b.weight > 0 ? '+' : ''}
                      {b.weight.toFixed(2)}
                    </span>
                    <span style={{ color: '#929b8b', fontSize: 12 }}>
                      半衰期 {b.halfLife.toFixed(0)}m · 已过 {b.age.toFixed(0)}m
                    </span>
                    <Btn onClick={() => unbias(b.tag)}>撤</Btn>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card title="调试">
            <Row>
              <Btn onClick={() => patch({ hunger: 10 })}>饿到 10</Btn>
              <Btn onClick={() => patch({ thirst: 10 })}>渴到 10</Btn>
              <Btn onClick={() => patch({ strength: 10 })}>累到 10</Btn>
              <Btn onClick={() => patch({ feeling: 10 })}>心情 10</Btn>
              <Btn onClick={() => patch({ health: 40 })}>病到 40</Btn>
              <Btn onClick={() => patch({ health: 15 })}>病到 15</Btn>
            </Row>
            <Row>
              <Btn onClick={() => patch({ money: 0 })}>钱清零</Btn>
              <Btn onClick={() => patch({ money: 5000 })}>给她 5000</Btn>
              <Btn onClick={() => patch({ affection: 0 })}>好感清零</Btn>
              <Btn onClick={() => patch({ affection: 100 })}>好感拉满</Btn>
              <Btn
                onClick={() =>
                  patch({ strength: 100, feeling: 60, hunger: 100, thirst: 100, affection: 50, health: 100 })
                }
              >
                全部复原
              </Btn>
            </Row>
          </Card>

          <Card title="工具测试">
            <Row>
              <Btn onClick={() => callTool('create_timer', { duration: '10s', label: '十秒到了' })}>
                测试 10 秒提醒
              </Btn>
              <Btn onClick={() => callTool('get_pet_state', {})}>读取当前状态</Btn>
              <Btn onClick={() => callTool('set_permission', { scope: 'kb.read', decision: 'allow' })}>
                测试权限拦截
              </Btn>
            </Row>
            {audit.length === 0 ? (
              <p className="status-empty">暂无调用记录</p>
            ) : (
              <ul style={{ listStyle: 'none', padding: 0, margin: 0, fontSize: 13 }}>
                {audit.map((a, i) => (
                  <li
                    key={`${a.at}-${i}`}
                    style={{ display: 'flex', gap: 10, padding: '7px 0', borderTop: '1px solid #e7e9df' }}
                  >
                    <span style={{ color: '#929b8b', fontVariantNumeric: 'tabular-nums' }}>
                      {new Date(a.at).toLocaleTimeString('zh-CN', { hour12: false })}
                    </span>
                    <span style={{ color: DECISION_COLOR[a.decision], width: 40 }}>{a.decision}</span>
                    <span style={{ width: 120 }}>{a.tool}</span>
                    <span style={{ flex: 1, color: '#929b8b', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {a.summary}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card title="番茄钟">
            {pomo ? (
              <>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 14, marginBottom: 12 }}>
                  <strong style={{ fontSize: 30, fontVariantNumeric: 'tabular-nums' }}>
                    {mmss(pomo.remainingSec)}
                  </strong>
                  <span style={{ color: pomo.phase === 'focus' ? '#748b68' : '#8b9c74' }}>
                    {PHASE_LABEL[pomo.phase]}
                  </span>
                  <span style={{ color: '#929b8b', fontSize: 13 }}>已完成 {pomo.completed} 个</span>
                </div>
                <Btn onClick={() => void invokeCore('stop_pomodoro').then(() => setPomo(null))}>停止</Btn>
              </>
            ) : (
              <>
                <Btn onClick={() => void invokeCore<Pomo>('start_pomodoro').then((p) => p && setPomo(p))}>
                  开始专注
                </Btn>
              </>
            )}
          </Card>

          <Card title="计时器">
            <Row>
              <Btn onClick={() => addTimer('10s', '十秒到了')}>10 秒后</Btn>
              <Btn onClick={() => addTimer('25m', '该休息了')}>25 分钟后</Btn>
              <Btn onClick={() => addTimer('1m', '每分钟提醒', true)}>每分钟</Btn>
            </Row>
            {timers.length === 0 ? (
              <p className="status-empty">暂无计时器</p>
            ) : (
              <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
                {timers.map((t) => (
                  <li
                    key={t.id}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 10,
                      padding: '7px 0', borderTop: '1px solid #e7e9df',
                    }}
                  >
                    <span style={{ flex: 1 }}>
                      {t.label}
                      {t.repeatMs != null && <span style={{ color: '#929b8b' }}> · 循环</span>}
                    </span>
                    <span style={{ color: '#929b8b', fontVariantNumeric: 'tabular-nums', fontSize: 13 }}>
                      {remaining(t.dueAt)}
                    </span>
                    <Btn onClick={() => void invokeCore('cancel_timer', { id: t.id }).then(pullTimers)}>
                      取消
                    </Btn>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </>
      )}
    </div>
  )
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="status-card">
      <h2>{title}</h2>
      {children}
    </section>
  )
}

const Row = ({ children }: { children: React.ReactNode }) => (
  <div className="status-actions">{children}</div>
)

function Btn({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button className="status-button" onClick={onClick}>
      {children}
    </button>
  )
}
