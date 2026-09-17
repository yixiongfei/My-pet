import { useEffect, useState } from 'react'
import { IS_TAURI } from '../body/ipc'
import { invokeStrict } from '../chat/api'
import type { ActionInfo, ChatSettings, LineMode, TtsStatus } from '../chat/api'
import { Icon } from '../chat/Icons'

/** 与 src-tauri/src/tts.rs 的 SPEAKERS 一致：名字 → 给人看的描述 */
const SPEAKERS: Array<[string, string]> = [
  ['serena', '温柔的年轻女声（默认）'], ['vivian', '明亮、带点俏皮的年轻女声'], ['ono_anna', '轻快活泼的日系女声'],
  ['sohee', '温暖、情绪丰富的韩系女声'], ['uncle_fu', '低沉醇厚的大叔声'], ['dylan', '清亮自然的北京男声'],
  ['eric', '带点沙哑的成都男声'], ['ryan', '有节奏感的英文男声'], ['aiden', '阳光的美式男声'],
]
const MODE_LABEL: Record<LineMode, string> = { fixed: '固定台词', model: '模型即兴', off: '不说话' }
const SAMPLE_LINE = '你好呀，我是{name}。今天也一起加油吧！Hello, nice to see you.'

interface Props {
  settings: ChatSettings
  setSettings: (update: (s: ChatSettings) => ChatSettings) => void
  busy: boolean
  /** 跑一个会改状态的操作：成功的提示 / 失败的错误由父组件显示 */
  action: (operation: () => Promise<string>) => Promise<void>
  /** 把当前设置存进 Core（会抛错，交给 action 包） */
  persist: () => Promise<void>
}

/**
 * 「声音与台词」：语音开关、声音和语气；动作开始时说的话——每个动作可以配多条，
 * 也可以让模型按人设写几条或者干脆每次即兴。
 */
export function VoiceSettingsTab({ settings, setSettings, busy, action, persist }: Props) {
  const [tts, setTts] = useState<TtsStatus | null>(null)
  const [actions, setActions] = useState<ActionInfo[]>([])
  const [editing, setEditing] = useState<string>('')
  const [drafting, setDrafting] = useState(false)
  const [previewing, setPreviewing] = useState(false)
  const voice = settings.voice
  const lines = settings.lines

  useEffect(() => {
    if (!IS_TAURI) return
    let live = true
    void invokeStrict<TtsStatus>('tts_status').then(s => live && setTts(s)).catch(() => {})
    void invokeStrict<ActionInfo[]>('list_actions').then(list => {
      if (!live) return
      setActions(list)
      setEditing(e => e || list.find(a => a.defaultLines.length)?.id || list[0]?.id || '')
    }).catch(() => {})
    return () => { live = false }
  }, [])

  const setVoice = (patch: Partial<ChatSettings['voice']>) => setSettings(s => ({ ...s, voice: { ...s.voice, ...patch } }))
  const setLines = (patch: Partial<ChatSettings['lines']>) => setSettings(s => ({ ...s, lines: { ...s.lines, ...patch } }))
  const current = actions.find(a => a.id === editing)
  const override = lines.actions[editing]
  const effectiveLines = override?.lines ?? current?.defaultLines ?? []
  const setActionLines = (id: string, text: string[] | null, mode?: LineMode | null) => setLines({
    actions: { ...lines.actions, [id]: { mode: mode === undefined ? (override?.mode ?? null) : mode, lines: text } },
  })

  const checkTts = () => action(async () => {
    const result = await invokeStrict<TtsStatus>('tts_status')
    setTts(result)
    if (!result.connected) throw new Error(result.error || '连不上语音服务。请用 scripts/start-vpet.ps1 启动，或先运行 scripts/setup-tts.ps1 安装。')
    return `语音服务已连接：${result.voices.length} 个声音可用。`
  })

  const save = () => action(async () => { await persist(); return '已保存。桌面上的她会立刻用新的声音。' })

  /** 试听：先保存（合成用的是 Core 里保存过的设置），再在这个窗口里播 */
  const preview = () => {
    if (previewing) return
    setPreviewing(true)
    void action(async () => {
      await persist()
      const buf = await invokeStrict<ArrayBuffer>('tts_speak', { text: SAMPLE_LINE.replace('{name}', settings.persona.name), mood: 'happy' })
      const url = URL.createObjectURL(new Blob([buf], { type: 'audio/wav' }))
      const audio = new Audio(url)
      audio.onended = () => URL.revokeObjectURL(url)
      await audio.play()
      return '正在试听。第一次合成要预热十几秒，之后会快很多。'
    }).finally(() => setPreviewing(false))
  }

  const draft = () => {
    if (!current || drafting) return
    setDrafting(true)
    void action(async () => {
      const fresh = await invokeStrict<string[]>('draft_action_lines', { id: current.id, count: 5 })
      setActionLines(current.id, [...effectiveLines, ...fresh])
      return `模型写了 ${fresh.length} 句，已加到「${current.name}」的台词里；不喜欢的删掉，记得保存。`
    }).finally(() => setDrafting(false))
  }

  return <>
    <div className="section-kicker">HER VOICE</div><h2>让她开口说话</h2>
    <p className="section-description">语音由本机的 Qwen3-TTS 合成（中英文都行），不联网。声音和语气在这里挑；桌面上的短句和对话回复都可以念出来。</p>

    <div className={`connection-card ${tts?.connected ? 'connected' : ''}`}>
      <span className={`status-dot ${tts?.connected ? 'is-online' : ''}`} />
      <div>
        <strong>{!IS_TAURI ? '浏览器预览' : tts?.connected ? '语音服务已连接' : '语音服务未连接'}</strong>
        <p>{!IS_TAURI ? '在桌宠应用中检查真实的连接状态。' : tts?.connected ? `Qwen3-TTS · ${tts.voices.length} 个声音` : tts?.error || '点击右侧按钮检查。首次使用请先运行 scripts/setup-tts.ps1。'}</p>
      </div>
      <button className="secondary-button" disabled={busy} onClick={() => void checkTts()}>检查连接</button>
    </div>

    <label className="toggle-row"><span><strong>开启语音</strong><small>关掉之后她只出气泡，不出声。</small></span><input type="checkbox" role="switch" checked={voice.enabled} onChange={e => setVoice({ enabled: e.target.checked })} /><span className="switch-track" /></label>
    <label className="toggle-row" style={{ borderTop: 0 }}><span><strong>念对话回复</strong><small>聊天窗口里的回答，桌面上的她也会念出来（按句合成，边说边念）。</small></span><input type="checkbox" role="switch" checked={voice.speakChat} onChange={e => setVoice({ speakChat: e.target.checked })} /><span className="switch-track" /></label>
    <label className="toggle-row" style={{ borderTop: 0 }}><span><strong>念桌面短句</strong><small>动作台词、答应或拒绝你、收到礼物时的那句话。</small></span><input type="checkbox" role="switch" checked={voice.speakLines} onChange={e => setVoice({ speakLines: e.target.checked })} /><span className="switch-track" /></label>

    <div className="field-columns" style={{ marginTop: 18 }}>
      <label className="form-field">声音
        <select value={voice.voice} onChange={e => setVoice({ voice: e.target.value })}>
          {SPEAKERS.map(([id, label]) => <option key={id} value={id}>{label} · {id}</option>)}
          {tts?.voices.filter(v => !SPEAKERS.some(([id]) => id === v)).map(v => <option key={v} value={v}>{v}</option>)}
        </select>
        <small>她是个温柔又好奇的少女，默认用 serena；想活泼一点换 vivian。</small>
      </label>
      <label className="form-field">语气说明<input value={voice.style} maxLength={200} placeholder="例如：语气自然亲切，像和熟人说话" onChange={e => setVoice({ style: e.target.value })} /><small>交给语音模型的风格指令，留空也可以。</small></label>
    </div>
    <label className="toggle-row"><span><strong>语气跟着心情走</strong><small>开心时轻快一点，累了、状态差的时候有气无力。</small></span><input type="checkbox" role="switch" checked={voice.moodStyle} onChange={e => setVoice({ moodStyle: e.target.checked })} /><span className="switch-track" /></label>
    <label className="field-label" htmlFor="voice-speed">语速<span>{voice.speed.toFixed(2)}×</span></label>
    <input id="voice-speed" className="range-input" type="range" min="0.5" max="2" step="0.05" value={voice.speed} onChange={e => setVoice({ speed: Number(e.target.value) })} />
    <div className="range-captions"><span>慢一点</span><span>快一点</span></div>
    <label className="form-field">语音服务地址<input value={voice.endpoint} maxLength={300} placeholder="http://127.0.0.1:8090" onChange={e => setVoice({ endpoint: e.target.value })} /><small>仅本机地址。scripts/start-vpet.ps1 默认在 8090 端口拉起 tts-server。</small></label>
    <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
      <button className="primary-button" disabled={busy} onClick={() => void save()}>{busy ? '保存中…' : '保存语音设置'}<Icon name="check" size={16} /></button>
      <button className="secondary-button" disabled={busy || previewing || !voice.enabled} onClick={preview}>{previewing ? '合成中…' : '保存并试听'}</button>
    </div>

    <div className="training-card" style={{ marginTop: 28 }}>
      <div className="section-kicker"><Icon name="spark" size={16} /> 动作台词</div>
      <h3>做事的时候，随口说一句</h3>
      <p>去工作、去吃饭、开始玩游戏……每个动作可以配多条台词，随机说一句；也可以让模型按她的性格写几条，或者每次都即兴。<code>{'{food}'}</code> 会替换成手里的东西，<code>{'{name}'}</code> 是她的名字。</p>
      <label className="toggle-row"><span><strong>开始做事时说一句</strong><small>关掉之后她默默做事，气泡和语音都不出。</small></span><input type="checkbox" role="switch" checked={lines.enabled} onChange={e => setLines({ enabled: e.target.checked })} /><span className="switch-track" /></label>
      <div className="field-columns">
        <label className="form-field">默认方式
          <select value={lines.mode} onChange={e => setLines({ mode: e.target.value as LineMode })}>
            <option value="fixed">固定台词（随机挑一句）</option>
            <option value="model">模型即兴（每次不一样，慢几秒）</option>
          </select>
          <small>单个动作可以在下面单独设置。</small>
        </label>
        <label className="form-field">两句之间至少隔<input type="number" min={0} max={3600} value={lines.minGapSec} onChange={e => setLines({ minGapSec: Math.max(0, Math.min(3600, Number(e.target.value) || 0)) })} /><small>秒。免得她换事情做得勤的时候像复读机。</small></label>
      </div>
      {actions.length ? <>
        <div className="field-columns">
          <label className="form-field">动作
            <select value={editing} onChange={e => setEditing(e.target.value)}>
              {actions.map(a => <option key={a.id} value={a.id}>{a.name}{lines.actions[a.id]?.lines ? ' · 已自定义' : ''}{lines.actions[a.id]?.mode === 'off' ? ' · 不说' : lines.actions[a.id]?.mode === 'model' ? ' · 即兴' : ''}</option>)}
            </select>
          </label>
          <label className="form-field">这个动作
            <select value={override?.mode ?? ''} onChange={e => setActionLines(editing, override?.lines ?? null, (e.target.value || null) as LineMode | null)}>
              <option value="">跟随默认（{MODE_LABEL[lines.mode]}）</option>
              <option value="fixed">固定台词</option>
              <option value="model">模型即兴</option>
              <option value="off">不说话</option>
            </select>
          </label>
        </div>
        <label className="form-field">「{current?.name}」的台词（一行一句）
          <textarea rows={5} value={effectiveLines.join('\n')} maxLength={4000} placeholder="一行一句。留空 = 这个动作没有固定台词" onChange={e => setActionLines(editing, e.target.value.split('\n'))} />
          <small>{override?.lines ? '已自定义。' : '正在用默认台词。'}模型即兴失败时也会退回这里的句子。</small>
        </label>
        <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', marginBottom: 12 }}>
          <button className="secondary-button" disabled={busy || drafting || !current} onClick={draft}>{drafting ? '模型在写…' : '让模型写几句'}</button>
          {override?.lines && <button className="secondary-button" disabled={busy} onClick={() => setActionLines(editing, null)}>恢复默认台词</button>}
        </div>
      </> : <p className="muted">{IS_TAURI ? '正在读取动作列表…' : '打开桌宠应用后，这里可以逐个动作配置台词。'}</p>}
      <button className="primary-button" disabled={busy} onClick={() => void save()}>{busy ? '保存中…' : '保存台词设置'}<Icon name="check" size={16} /></button>
    </div>
  </>
}
