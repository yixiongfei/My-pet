import { invokeCore } from './ipc'

export type SpeechKind = 'chat' | 'line'

export interface VoicePrefs {
  enabled: boolean
  speakChat: boolean
  speakLines: boolean
  /** 播放倍速。tts-server 不做变速，在这里做；不保持音高时快一点也高一点 */
  speed: number
  keepPitch: boolean
}

interface Item {
  kind: SpeechKind
  audio: Promise<Blob | null>
}

/** 句末：到这里就可以先送去合成，不用等整段回复说完 */
const SENTENCE_END = /[。！？!?；;\n]|…+/g
/** 太短的碎片（「嗯。」）攒到下一句一起念，省一次合成 */
const MIN_SENTENCE_CHARS = 4

/**
 * 说话队列。一次只出一个声音；按句排队，前一句在播的时候后一句已经在合成了——
 * 合成一句四秒的话要三秒左右（核显），整段回复一起合成会等太久。
 * 合成在 Core（`tts_speak`，带磁盘缓存），这里只管顺序和播放。
 */
export class Speech {
  prefs: VoicePrefs = { enabled: true, speakChat: true, speakLines: true, speed: 1.12, keepPitch: false }
  /** 开口 / 闭嘴，给 say 动画用 */
  onTalking: ((talking: boolean) => void) | null = null
  /** 队列空了（不管有没有真的出过声）：气泡可以收了 */
  onIdle: (() => void) | null = null

  private queue: Item[] = []
  private current: HTMLAudioElement | null = null
  private currentUrl: string | null = null
  private pumping = false
  private talking = false
  private disposed = false

  allows(kind: SpeechKind): boolean {
    if (!this.prefs.enabled) return false
    return kind === 'chat' ? this.prefs.speakChat : this.prefs.speakLines
  }

  /**
   * 排一句。`interrupt` = 把还没说完的都扔掉（新的回复来了，旧的就别念了）。
   * 不允许出声的类型直接忽略——气泡照出，只是没声音
   */
  say(text: string, kind: SpeechKind, opts: { interrupt?: boolean; mood?: string } = {}): void {
    if (this.disposed || !this.allows(kind)) return
    const clean = text.trim()
    if (!clean) return
    if (opts.interrupt) this.interrupt()
    // 台词类的话别打断正在念的对话回复，也别排一长串：正在说就算了
    if (kind === 'line' && (this.current || this.queue.length)) return
    const audio = invokeCore<ArrayBuffer>('tts_speak', { text: clean, mood: opts.mood })
      .then((buf) => (buf && buf.byteLength > 44 ? new Blob([buf], { type: 'audio/wav' }) : null))
      .catch(() => null)
    this.queue.push({ kind, audio })
    void this.pump()
  }

  /** 把一段流式文本按句切开：返回可以先念的整句，剩下的留给下一次 */
  static splitSentences(text: string): { ready: string[]; rest: string } {
    const ready: string[] = []
    let start = 0
    let pending = ''
    for (const m of text.matchAll(SENTENCE_END)) {
      const end = m.index + m[0].length
      const piece = pending + text.slice(start, end)
      start = end
      if (piece.trim().length < MIN_SENTENCE_CHARS) { pending = piece; continue }
      ready.push(piece)
      pending = ''
    }
    return { ready, rest: pending + text.slice(start) }
  }

  interrupt(): void {
    this.queue = []
    this.stopCurrent()
    this.setTalking(false)
  }

  dispose(): void {
    this.disposed = true
    this.interrupt()
  }

  private stopCurrent(): void {
    if (this.current) {
      this.current.onended = null
      this.current.onerror = null
      this.current.pause()
      this.current = null
    }
    if (this.currentUrl) {
      URL.revokeObjectURL(this.currentUrl)
      this.currentUrl = null
    }
  }

  private setTalking(on: boolean): void {
    if (this.talking === on) return
    this.talking = on
    this.onTalking?.(on)
  }

  private async pump(): Promise<void> {
    if (this.pumping) return
    this.pumping = true
    try {
      while (!this.disposed && this.queue.length) {
        const item = this.queue[0]
        const blob = await item.audio
        // 等合成的这段时间里可能被 interrupt 清空了
        if (this.disposed || this.queue[0] !== item) continue
        this.queue.shift()
        if (!blob) continue
        await this.play(blob)
      }
    } finally {
      this.pumping = false
      if (!this.queue.length) {
        this.setTalking(false)
        this.onIdle?.()
      }
    }
  }

  private play(blob: Blob): Promise<void> {
    return new Promise((resolve) => {
      this.stopCurrent()
      const url = URL.createObjectURL(blob)
      const audio = new Audio(url)
      audio.playbackRate = Math.min(1.6, Math.max(0.7, this.prefs.speed || 1))
      audio.preservesPitch = this.prefs.keepPitch
      this.current = audio
      this.currentUrl = url
      const done = () => {
        if (this.current === audio) this.stopCurrent()
        resolve()
      }
      audio.onended = done
      audio.onerror = done
      this.setTalking(true)
      audio.play().catch((e: unknown) => {
        console.warn('[VPet] 播放语音失败', e)
        done()
      })
    })
  }
}
