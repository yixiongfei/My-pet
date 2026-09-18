import { Verdict } from '@vpet/shared'
import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimationPlayer } from './AnimationPlayer'
import { Bubble, themeNow, type BubbleTheme } from './Bubble'
import { Countdown } from './Countdown'
import { subscribe } from './events'
import { HitMask } from './hitMask'
import { Interaction } from './interaction'
import { invokeCore } from './ipc'
import { loadManifest, loadProfile } from './manifest'
import { fetchPetState, subscribePetState } from './petState'
import { exitPetSide, openChat, openSettingsPanel, pushFoodCatalog, pushHitMask, pushMotionProfile, reportTouch } from './petWindow'
import { hideDelayMs } from './say'
import { Speech, type SpeechCue } from './speech'
import { toLogical } from './touch'

/** 话在念的时候气泡不收；念完再留一会儿。合成加播放最长也就这么久，兜底 */
const SPEAKING_HOLD_MS = 45_000
const AFTER_SPEECH_MS = 2_000
/** 她主动搭话你没回：三十秒就收，不追着问 */
const NUDGE_HOLD_MS = 30_000
/** 自言自语没出声时最多留这么久——余光扫到就行 */
const SELF_TALK_HOLD_MS = 6_000
/** 立绘上方留给气泡 / 倒计时的高度，按立绘边长算。**必须和 src-tauri lib.rs 的 HEAD_ROOM 一致**（窗口高 = 宽 × 1.6） */
const HEAD_ROOM = 0.6

/**
 * 各类话配哪套 say 动画（Say/Self · Serious · Shining · Shy）。
 * 自言自语 → self；日程提醒这类正经话 → serious；收礼 / 吃药的道谢 → shy；
 * 普通动作台词 → self（那是她随口说的，不是对你说的）
 */
function sayStyleForLine(line: { level?: string; action?: string }): string {
  if (line.level === 'self') return 'self'
  if (line.action === 'nudge') return 'serious'
  if (line.action === 'gift' || line.action === 'medicine') return 'shy'
  return 'self'
}

/** 对话回复：句子里有害羞 / 道谢 / 亲昵的字眼就 shy，其余按心情（Interaction 里挑） */
function sayStyleForChat(sentence: string): string | undefined {
  return /谢谢|害羞|喜欢你|亲|抱|脸红|讨厌啦|才不是/.test(sentence) ? 'shy' : undefined
}

/** 与 core/scheduler.rs 的 Timer 对应；focus 非空才在头顶显示 */
interface FocusTimer { id: string; label: string; dueAt: number; focus?: { startedAt: number; target: string | null } | null }

/** Core 那边的 ChatSettings 里我们只关心这几个开关 */
interface VoiceSettingsPayload {
  voice?: { enabled?: boolean; speakChat?: boolean; speakLines?: boolean; speed?: number; keepPitch?: boolean }
  persona?: { name?: string }
}

export function PetCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const maskRef = useRef<HitMask | null>(null)
  const interactionRef = useRef<Interaction | null>(null)
  const speechRef = useRef<Speech | null>(null)
  const hideTimer = useRef(0)
  /** 当前气泡是什么时候、为哪段话弹出来的：念完之后决定还留多久 */
  const bubbleMeta = useRef<{ at: number; text: string }>({ at: 0, text: '' })
  const [error, setError] = useState<string | null>(null)
  const [name, setName] = useState('VPet')
  const [bubble, setBubble] = useState<string | null>(null)
  const [bubbleTone, setBubbleTone] = useState<'talk' | 'self'>('talk')
  /** 白天白气泡、黑夜黑气泡；每分钟看一次钟 */
  const [theme, setTheme] = useState<BubbleTheme>(() => themeNow())
  useEffect(() => {
    const t = window.setInterval(() => setTheme(themeNow()), 60_000)
    return () => window.clearInterval(t)
  }, [])
  const [streaming, setStreaming] = useState(false)
  /** 正在流式收的那条回复：文本 + 已经送去念到哪了 */
  const streamRef = useRef<{ id: string; text: string; spoken: number; speech?: SpeechCue } | null>(null)
  const [hovered, setHovered] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [focus, setFocus] = useState<FocusTimer | null>(null)

  const scheduleHide = useCallback((ms: number) => {
    window.clearTimeout(hideTimer.current)
    hideTimer.current = window.setTimeout(() => setBubble(null), ms)
  }, [])

  /**
   * 桌面上的短句：气泡 + （开了语音就）念出来。念的话气泡等念完再收。
   * `level` 是分量：自言自语小气泡、轻声、早收；搭话（`nudge`）没回应三十秒就收
   */
  const announce = useCallback((text: string, speak: 'line' | 'none' = 'line', opts: { level?: 'talk' | 'self'; volume?: number; nudge?: boolean; style?: string } = {}) => {
    streamRef.current = null
    setStreaming(false)
    const level = opts.level ?? 'talk'
    setBubbleTone(level)
    setBubble(text)
    bubbleMeta.current = { at: Date.now(), text }
    const speech = speechRef.current
    const spoken = speak === 'line' && !!speech?.allows('line')
    const hold = level === 'self' ? Math.min(SELF_TALK_HOLD_MS, hideDelayMs(text)) : opts.nudge ? Math.min(NUDGE_HOLD_MS, hideDelayMs(text)) : hideDelayMs(text)
    scheduleHide(spoken ? SPEAKING_HOLD_MS : hold)
    if (spoken) speech?.say(text, 'line', { volume: opts.volume, style: opts.style })
  }, [scheduleHide])

  /** 语音开关变了（设置页保存）就同步给说话队列 */
  const applyVoiceSettings = useCallback((payload: unknown) => {
    const s = payload as VoiceSettingsPayload | null
    const speech = speechRef.current
    if (!s || !speech) return
    speech.prefs = {
      enabled: s.voice?.enabled ?? true,
      speakChat: s.voice?.speakChat ?? true,
      speakLines: s.voice?.speakLines ?? true,
      speed: s.voice?.speed ?? 1.12,
      keepPitch: s.voice?.keepPitch ?? false,
    }
    if (s.persona?.name) setName(s.persona.name)
  }, [])

  /**
   * 聊天窗口里她在说什么，桌面上的她也同步「说」出来：流式气泡，并且按句送去合成——
   * 前一句在念的时候后一句已经在合成了，不用等整段回复结束
   */
  const onChatStream = useCallback((payload: unknown) => {
    const event = payload as { requestId?: string; delta?: string; done?: boolean; text?: string; reset?: boolean; speech?: SpeechCue } | null
    if (!event?.requestId) return
    const speech = speechRef.current
    let current = streamRef.current
    if (event.reset && current?.id === event.requestId) {
      // 模型重来了一次：已经念出去的作废，气泡清空
      speech?.interrupt()
      interactionRef.current?.startThink()
      current = null
      streamRef.current = null
      setBubble(null)
    }
    if (event.done) {
      interactionRef.current?.endThink()
      if (current?.id !== event.requestId) return
      streamRef.current = null
      setStreaming(false)
      // Core 收尾时会把清理过的最终文本带回来（去掉模型拖出的旁白），以它为准
      const text = (event.text ?? current.text).trim()
      if (!text) { setBubble(null); speech?.interrupt(); return }
      setBubbleTone('talk')
      setBubble(text)
      bubbleMeta.current = { at: Date.now(), text }
      const spoken = !!speech?.allows('chat')
      scheduleHide(spoken ? SPEAKING_HOLD_MS : hideDelayMs(text))
      // 还没念的尾巴：最终文本和流式文本的开头一致才接着念，否则（旁白被裁掉了）只念还没念过的部分
      const spokenPrefix = current.text.slice(0, current.spoken)
      const rest = text.startsWith(spokenPrefix) ? text.slice(current.spoken) : current.spoken === 0 ? text : ''
      if (rest.trim()) speech?.say(rest, 'chat', { style: sayStyleForChat(text), speech: event.speech ?? current.speech })
      return
    }
    window.clearTimeout(hideTimer.current)
    const fresh = !current || current.id !== event.requestId
    if ((event.delta ?? '').length > 0) interactionRef.current?.endThink()
    if (fresh) speech?.interrupt() // 新的回复来了，旧的别念了
    const next = !current || fresh
      ? { id: event.requestId, text: event.delta ?? '', spoken: 0, speech: event.speech }
      : { id: event.requestId, text: current.text + (event.delta ?? ''), spoken: current.spoken, speech: event.speech ?? current.speech }
    const { ready } = Speech.splitSentences(next.text.slice(next.spoken))
    for (const sentence of ready) {
      speech?.say(sentence, 'chat', { style: sayStyleForChat(sentence), speech: next.speech })
      next.spoken += sentence.length
    }
    streamRef.current = next
    setStreaming(true)
    setBubbleTone('talk')
    setBubble(next.text)
  }, [scheduleHide])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    let disposed = false
    let player: AnimationPlayer | null = null
    const stops: Array<() => void> = []
    const speech = new Speech()
    speechRef.current = speech

    Promise.all([loadManifest(), loadProfile()])
      .then(([manifest, profile]) => {
        if (disposed) return
        setName(profile.name)
        player = new AnimationPlayer(canvas, manifest)
        // Preserve source resolution; fit viewport at every desktop size.
        canvas.style.width = '100%'
        canvas.style.height = '100%'
        const mask = new HitMask()
        maskRef.current = mask
        player.onFrame = (composited) => {
          const changed = mask.update(composited)
          if (changed) pushHitMask(changed)
        }
        const interaction = new Interaction({
          player, manifest, profile,
          onTouch: reportTouch,
          onClick: () => void openChat(),
          onDragChange: setDragging,
          onSideExit: exitPetSide,
        })
        interactionRef.current = interaction
        interaction.start()
        // 出声的时候播 say 动画，说完回到手头的事；念完了气泡再留两秒
        speech.onTalking = (talking, style) => (talking ? interaction.startSay(style) : interaction.endSay())
        speech.onIdle = () => {
          if (streamRef.current) return
          // 没出声（服务没起来）也得让人把字看完：按字数的时长和「念完再留两秒」取长的
          const { at, text } = bubbleMeta.current
          scheduleHide(Math.max(AFTER_SPEECH_MS, hideDelayMs(text) - (Date.now() - at)))
        }
        pushFoodCatalog(manifest.food)
        pushMotionProfile({ moves: profile.moves, side: profile.side })
        stops.push(subscribePetState((state) => interaction.setState(state)))
        void fetchPetState().then((state) => { if (state && !disposed) interaction.setState(state) })
        void invokeCore<unknown>('get_chat_settings').then((s) => { if (!disposed) applyVoiceSettings(s) })
        stops.push(subscribe('chat:settings-changed', applyVoiceSettings))
        stops.push(subscribe('pet:prompt', () => void openChat()))
        stops.push(subscribe('chat:thinking', (payload) => {
          const event = payload as { active?: boolean } | null
          if (event?.active) {
            speech.interrupt()
            interaction.startThink()
          }
          else interaction.endThink()
        }))
        stops.push(subscribe('chat-stream', onChatStream))
        stops.push(subscribe('pet:drag-ended', () => interaction.onPointerUp(true)))
        stops.push(subscribe('pet:motion', (payload) => interaction.handleMotion(payload)))
        // 歌到高潮：跳舞换成 ohhhh
        stops.push(subscribe('pet:music', (payload) => {
          const m = payload as { playing?: boolean; climax?: boolean } | null
          interaction.setClimax(!!m?.playing && !!m?.climax)
        }))
        // 托盘“退出”先让她播退场，动画结束才真正退出；Core 有 8 秒超时兜底。
        stops.push(subscribe('pet:shutdown-requested', () => {
          speech.interrupt()
          interaction.playShutdown(() => { void invokeCore('finish_shutdown') })
        }))
        // 拆礼物的动画只播一遍；她说什么由紧跟着的 pet:line 决定
        stops.push(subscribe('pet:gift', (payload) => {
          const received = payload as { id?: string } | null
          interaction.playGift(received?.id)
        }))
        // 动作台词：开始做一件事时随口一句（settings 里可以关、可以改）
        stops.push(subscribe('pet:line', (payload) => {
          const line = payload as { text?: string; spoken?: boolean; level?: string; volume?: number; action?: string } | null
          if (line?.text) {
            announce(line.text, line.spoken === false ? 'none' : 'line', {
              level: line.level === 'self' ? 'self' : 'talk',
              volume: line.volume,
              nudge: line.action === 'nudge',
              style: sayStyleForLine(line),
            })
          }
        }))
        stops.push(subscribe('timer:fired', (payload) => {
          const t = payload as FocusTimer | null
          if (!t?.label) return
          announce(t.focus ? `⏰ 「${t.label}」到啦！` : '⏰ ' + t.label)
        }))
        // 头顶的倒计时：番茄钟 / 「学习一个小时」。重启后接着显示
        void invokeCore<FocusTimer | null>('get_focus').then((t) => { if (!disposed) setFocus(t?.focus ? t : null) })
        stops.push(subscribe('focus:started', (payload) => {
          const t = payload as FocusTimer | null
          setFocus(t?.focus ? t : null)
        }))
        stops.push(subscribe('focus:ended', () => setFocus(null)))
        stops.push(subscribe('pet:said', (payload) => {
          const result = Verdict.safeParse(payload)
          if (result.success) announce(result.data.say)
        }))
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))

    const cancel = () => interactionRef.current?.onPointerUp(true)
    window.addEventListener('blur', cancel)
    return () => {
      disposed = true
      stops.forEach((stop) => stop())
      window.removeEventListener('blur', cancel)
      window.clearTimeout(hideTimer.current)
      speech.dispose()
      speechRef.current = null
      interactionRef.current?.dispose()
      interactionRef.current = null
      maskRef.current = null
      player?.destroy()
    }
  }, [announce, applyVoiceSettings, onChatStream, scheduleHide])

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    const point = toLogical(event.currentTarget, event.clientX, event.clientY)
    if (maskRef.current && !maskRef.current.isOpaqueAt(point.x, point.y)) return
    event.currentTarget.setPointerCapture(event.pointerId)
    interactionRef.current?.onPointerDown(point.x, point.y, event.screenX, event.screenY)
  }

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const point = toLogical(event.currentTarget, event.clientX, event.clientY)
    const opaque = maskRef.current?.isOpaqueAt(point.x, point.y) ?? true
    setHovered(opaque)
    interactionRef.current?.setHovered(opaque)
    interactionRef.current?.onPointerMove(point.x, point.y, event.screenX, event.screenY)
  }

  const onPointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    interactionRef.current?.onPointerUp()
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
  }

  const closeBubble = () => {
    setBubble(null)
    speechRef.current?.interrupt()
  }

  // 窗口 = 上面一段头顶区（气泡、倒计时，鼠标穿透）+ 下面一个正方形的立绘区。
  // 两块的比例由 Rust 定（窗口高 = 宽 × (1 + HEAD_ROOM)），这里只是按同一个比例分
  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
      <div style={{ position: 'relative', flex: 'none', height: `calc(100vw * ${HEAD_ROOM})`, pointerEvents: 'none' }}>
        {focus?.focus && (
          <div style={bubble
            ? { position: 'absolute', top: 2, right: 6, zIndex: 2 }
            : { position: 'absolute', top: 4, left: 0, right: 0, display: 'flex', justifyContent: 'center' }}
          >
            <Countdown label={focus.label} startedAt={focus.focus.startedAt} dueAt={focus.dueAt} compact={!!bubble} />
          </div>
        )}
        {bubble && (
          <div style={{ position: 'absolute', left: 6, right: 6, bottom: 2 }}>
            <Bubble name={name} text={bubble} streaming={streaming} tone={bubbleTone} theme={theme} maxHeight={window.innerWidth * HEAD_ROOM - 6} />
          </div>
        )}
      </div>
      <div
        style={{ position: 'relative', width: '100vw', height: '100vw', touchAction: 'none', cursor: dragging ? 'grabbing' : hovered ? 'pointer' : 'default' }}
        title="单击聊天 · 按住拖动 · 右键打开设置"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => interactionRef.current?.onPointerUp(true)}
        onLostPointerCapture={() => interactionRef.current?.onPointerUp(true)}
        onPointerLeave={() => { setHovered(false); interactionRef.current?.setHovered(false) }}
        onContextMenu={(event) => { event.preventDefault(); void openSettingsPanel() }}
        onDoubleClick={closeBubble}
      >
        <canvas ref={canvasRef} />
        {error && (
          <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', padding: 24, textAlign: 'center', font: '14px/1.6 system-ui, sans-serif', color: '#fff', background: 'rgba(0,0,0,.72)', borderRadius: 16 }}>
            <div><div style={{ fontSize: 28, marginBottom: 8 }}>VPet</div>{error}</div>
          </div>
        )}
      </div>
    </div>
  )
}
