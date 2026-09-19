import type { GraphType, Manifest, PetProfile, PetState } from '@vpet/shared'
import type { AnimationPlayer } from './AnimationPlayer'
import type { SpeechCue } from './speech'
import { ACTION_POOLS, ANIMATION_POOLS, dwellMs, pickEntry, type PoolEntry } from './animationPool'
import { namesFor, pick, resolveClips, resolveLayered } from './manifest'
import { CLIP_FOR, DEFAULT_PET_STATE } from './petState'
import {
  beginPetDrag,
  beginPetMotionStep,
  completePetMotionCycle,
  endPetDrag,
  setHitTestPinned,
  startPetMotion,
  stopPetMotion,
  type PetMotionEvent,
} from './petWindow'
import { clickZone, inPinchZone, pressZone, raiseVariant, type ClickZone } from './touch'

/** 按住多久算长按（原版 Setting.PressLength，默认 0.5s） */
const PRESS_MS = 500
const DRAG_THRESHOLD_PX = 6
/** 空闲多久随机播一个小动作；缩短站立等待，保持角色有可见反馈。 */
const IDLE_ACTION_EVERY_MS: [number, number] = [8_000, 20_000]
/** 空闲动作中自主移动的占比；避免长期停在原地。 */
const MOVE_CHANCE = 0.35
/** 没去移动时进入 StateONE → StateTWO 待机姿态的概率，降低连续站立感。 */
const STATE_IDLE_CHANCE = 0.2
/** 提起后先挣扎一次再转静止，避免用户必须长按数秒才能看到 raised_static。 */
const STRUGGLE_TIMES = 1
/** 空闲小动作里播放庆祝（BDay）的概率。单独拆出，避免一直被 MI / MU 抽走。 */
const CELEBRATION_CHANCE = 0.18
/** 空闲小动作里插日常（Relax 的 MI / MU）的概率 */
const RELAX_CHANCE = 0.28
const RELAX_NAMES = ['mi', 'mu']
/** MI / MU 至少循环一段时间，避免伸懒腰或喝茶刚开始就结束。 */
const RELAX_PLAY_MS: [number, number] = [8_000, 12_000]
/** 全身状态都很好时，空闲小动作有这个概率变成飞吻（WORK/kiss） */
const KISS_CHANCE = 0.2
/** 爱意台词的最短间隔（毫秒）；庆祝动作会频繁抽样，不能每次都播台词。 */
const LOVE_LINE_COOLDOWN_MS = 5 * 60_000
/** 「全面状态都很高」的门槛 */
const KISS_MIN = { strength: 80, feeling: 80, hunger: 70, thirst: 70, health: 80, affection: 60 }
/** 吃饭时有这个概率是在吃麦当劳（Eat/EatMcDonald，不用夹心） */
const MCDONALD_CHANCE = 0.2
/** 启动后检查的节日；日期按本机时区判断，避免跨时区时祝福日期漂移。 */
const SPECIAL_DAYS: Array<{
  month: number
  day: number
  key: string
  type: GraphType
  name: string
  foodId?: string
  text: string
  speech: SpeechCue
}> = [
  { month: 1, day: 1, key: 'new-year', type: 'startup' as GraphType, name: 'newyear', text: '新年快乐！今年也一起好好生活吧。', speech: { emotion: 'excited' as const, energy: 0.82, style: 'playful' as const } },
  { month: 2, day: 14, key: 'valentines-day', type: 'work' as GraphType, name: 'kiss', text: '情人节快乐，送你一个喜欢的飞吻。', speech: { emotion: 'shy' as const, energy: 0.72, style: 'warm' as const } },
  { month: 6, day: 7, key: 'user-birthday', type: 'common' as GraphType, name: 'bday', foodId: 'birthday-cake', text: '生日快乐！今天要把开心和好运都收好哦。', speech: { emotion: 'happy' as const, energy: 0.8, style: 'playful' as const } },
  { month: 8, day: 14, key: 'pet-birthday', type: 'common' as GraphType, name: 'bday', foodId: 'birthday-cake', text: '今天是我的生日，也想把快乐分给你。', speech: { emotion: 'happy' as const, energy: 0.78, style: 'warm' as const } },
  { month: 12, day: 25, key: 'christmas', type: 'common' as GraphType, name: 'bday', text: '圣诞快乐！愿今天有礼物，也有好心情。', speech: { emotion: 'happy' as const, energy: 0.74, style: 'warm' as const } },
]
/** 说话动画：各种话配哪套 say。self 自言自语 · serious 正经提醒 · shining 开心 · shy 害羞 */
export type SayStyle = 'self' | 'serious' | 'shining' | 'shy'
const MOOD_RANK: Record<PetState['mood'], number> = { ill: 0, poorcondition: 1, nomal: 2, happy: 3 }

type InteractionMode =
  | 'idle' | 'touching' | 'raised' | 'talking' | 'moving' | 'move-exit' | 'side' | 'side-exit'
  | 'startup' | 'shutdown' | 'thinking' | 'transition' | 'idle-state-one' | 'idle-state-two' | 'pinching'

export interface InteractionOpts {
  player: AnimationPlayer
  manifest: Manifest
  profile: PetProfile
  /** 交互发生时通知外部；Phase 2 起转给 Core 的状态机改体力/心情 */
  onTouch?: (zone: ClickZone | 'raise') => void
  onClick?: () => void
  onDragChange?: (dragging: boolean) => void
  /** 侧挂的角色被碰到：先让 Core 把窗口完整拉回屏幕 */
  onSideExit?: () => void
  /** 开心时的庆祝 / 飞吻台词；由 Body 统一负责气泡和 TTS。 */
  onAmbientLine?: (text: string, speech: SpeechCue) => void
}

/**
 * 触摸交互状态机，移植自 legacy/VPet-Simulator.Core/Display/Main.xaml.cs 的鼠标处理：
 *
 *   短按（< PRESS_MS）命中头/身体 → 摸头 / 摸身体，三段式播完回当前活动
 *   长按（≥ PRESS_MS）命中提起区 → 挣扎×3 → 静止循环，窗口跟着光标走
 *   松手                          → 当前段播完 → 落地 → 回当前活动
 *
 * 判定用的是松手/长按那一刻的光标位置，不是按下的位置（与原版一致）。
 */
export class Interaction {
  /** 交互模式，和 PetState.activity 是两回事：这个说「此刻正在干什么」 */
  private mode: InteractionMode = 'idle'
  private state: PetState = DEFAULT_PET_STATE
  private idleTimer = 0
  private pressTimer = 0
  private lastAt: { x: number; y: number } | null = null
  private pressAt: { x: number; y: number; screenX: number; screenY: number } | null = null
  private struggles = 0
  private released = false
  private raiseName = 'raise'
  private raiseVariant: 0 | 1 = 0
  private pinned = false
  private disposed = false
  /** 正在出声（语音在放）。一次性动画播完后 toActivity 会先接上 say 动画 */
  private speaking = false
  /** 这句话配哪套 say；没指定按心情挑 */
  private sayStyle: SayStyle | undefined
  /** 按下的位置在脸上：拖起来是捏脸不是提起 */
  private pressOnFace = false
  /** 这一顿饭是不是在吃麦当劳（进入吃饭时掷一次，整顿饭不变） */
  private mcdonald = false
  /** 歌到高潮了：跳舞时换成 saraburate/ohhhh，过去了回到原来那段 */
  private climax = false
  /** 动画池里当前这一段：属于哪个活动、播到什么时候换。摸头 / 说话打断后回来接着播它 */
  private pooled: { activity: PetState['activity']; entry: PoolEntry; until: number } | null = null
  private poolTimer = 0
  private side: 'left' | 'right' | null = null
  private sideHovered = false
  /** 每次切换侧挂阶段就递增；异步解码完的旧回调看到代数不一致便作废 */
  private sideGeneration = 0
  private moveId: number | null = null
  private moveGraph: string | null = null
  /** Core 切换移动规则或用户中断时，让旧动画 / IPC 回调作废。 */
  private moveGeneration = 0
  /** 退出动画完成后通知 Core 真正结束进程；Rust 另有超时兜底。 */
  private shutdownDone: (() => void) | null = null
  private lastLoveLineAt = 0
  private specialDayPlayed = false
  private specialFoodId: string | null = null
  /** 上次播出的爱意台词，防止不同触发路径连续说同一句。 */
  private lastLoveLineText: string | null = null

  constructor(private readonly o: InteractionOpts) {}

  /** 开始播当前活动对应的动画 + 空闲小动作循环 */
  start(): void {
    this.o.player.onIdle = () => this.onAnimationIdle()
    if (this.hasClip('startup', 'startup')) {
      this.mode = 'startup'
      void this.o.player.playOnce({ type: 'startup', name: 'startup', mood: this.state.mood })
    } else if (!this.playSpecialDay()) this.toActivity()
  }

  /**
   * Core 推来新状态。正在摸 / 提起时不打断，等这段交互结束后
   * toActivity() 自然会用上新状态。
   */
  setState(s: PetState): void {
    if (this.disposed) return
    const before = this.state
    const changed =
      s.activity !== before.activity ||
      s.mood !== before.mood ||
      s.action?.id !== before.action?.id ||
      s.action?.food?.id !== before.action?.food?.id
    // 刚开始吃一顿饭：掷一次是不是麦当劳，整顿饭不变
    if (s.activity === 'eating' && (before.activity !== 'eating' || s.action?.id !== before.action?.id)) {
      this.mcdonald = s.action?.id !== 'medicine' && Math.random() < MCDONALD_CHANCE && this.hasClip('common', 'eatmcdonald')
    }
    this.state = s
    if (this.mode === 'shutdown') return
    if (changed && (this.mode === 'moving' || this.mode === 'move-exit')) {
      this.cancelMove()
      this.toActivity()
      return
    }
    const urgentActivity = s.activity !== before.activity &&
      (s.activity === 'eating' || s.activity === 'drinking' || s.activity === 'sleeping')
    const ordinaryVisual = this.mode === 'thinking' || this.mode === 'transition' ||
      this.mode === 'idle-state-one' || this.mode === 'idle-state-two'
    if (changed && urgentActivity && ordinaryVisual) {
      if (s.activity === 'drinking') this.playTransition('switch_thirsty', 'switch_thirsty')
      else if (s.activity === 'eating' && s.action?.id !== 'medicine') this.playTransition('switch_hunger', 'switch_hunger')
      else this.toActivity()
      return
    }
    if (!changed || this.mode !== 'idle') return

    // 生理需要的过场先于普通状态切换：她先表现「饿 / 渴」，再进入夹心吃喝动画。
    if (s.activity !== before.activity && s.activity === 'drinking') {
      this.playTransition('switch_thirsty', 'switch_thirsty')
    } else if (s.activity !== before.activity && s.activity === 'eating' && s.action?.id !== 'medicine') {
      this.playTransition('switch_hunger', 'switch_hunger')
    } else if (s.level > before.level && before.updatedAt !== 0) {
      this.playTransition('common', 'levelup')
    } else if (s.activity === before.activity && s.mood !== before.mood) {
      const type = MOOD_RANK[s.mood] > MOOD_RANK[before.mood] ? 'switch_up' : 'switch_down'
      this.playTransition(type, type)
    } else this.toActivity()
  }

  /**
   * 说话期间循环播 say 动画。正在摸 / 拆礼物这种一次性动画不打断——
   * 播完之后 toActivity 看到 speaking 还挂着，会接上 say。提起时不说话（嘴被拎着呢）
   */
  startSay(style?: string): void {
    if (this.disposed) return
    const next = (['self', 'serious', 'shining', 'shy'] as const).find((s) => s === style)
    // 已经在说了、只是换了一句：风格变了就换 say 动画，没变就接着播
    if (this.speaking && this.mode === 'talking') {
      if (next && next !== this.sayStyle) {
        this.sayStyle = next
        this.playSay()
      }
      return
    }
    this.sayStyle = next
    this.speaking = true
    if (this.mode === 'moving' || this.mode === 'move-exit') {
      this.cancelMove()
      this.playSay()
      return
    }
    if (this.mode === 'thinking') {
      // 首句语音准备好时结束思考；C 段播完后 toActivity 会看到 speaking 并接 say。
      this.o.player.stop()
      return
    }
    if (this.mode === 'idle') this.playSay()
  }

  /** 说完了，回到当前活动 */
  endSay(): void {
    if (this.disposed) return
    this.speaking = false
    if (this.mode === 'talking') this.toActivity()
  }

  private playSay(): void {
    this.mode = 'talking'
    window.clearTimeout(this.idleTimer)
    const style = this.sayStyle ?? this.sayStyleByMood()
    const name = this.hasClip('say', style) ? style : this.nameFor('say')
    void this.o.player.play({ type: 'say', name, mood: this.state.mood })
  }

  /** 没指定风格的话（普通对话）：开心就 shining，状态差就 serious，一般时 shining / self 随机 */
  private sayStyleByMood(): SayStyle {
    switch (this.state.mood) {
      case 'happy': return 'shining'
      case 'poorcondition':
      case 'ill': return 'serious'
      default: return Math.random() < 0.5 ? 'shining' : 'self'
    }
  }

  /** 模型收到问题、还没吐出首字时循环思考；首字或取消到来后自然播 C 段。 */
  startThink(): void {
    if (this.disposed || this.mode === 'shutdown' || this.mode === 'raised') return
    if (this.mode === 'moving' || this.mode === 'move-exit') this.cancelMove()
    if (this.mode !== 'idle' && this.mode !== 'thinking') return
    if (!this.hasClip('common', 'think')) return
    this.mode = 'thinking'
    window.clearTimeout(this.idleTimer)
    void this.o.player.play({ type: 'common', name: 'think', mood: this.state.mood })
  }

  endThink(): void {
    if (!this.disposed && this.mode === 'thinking') this.o.player.stop()
  }

  /** 托盘退出先播退场；结束回调再让 Rust 退出，超时由 Rust 自己兜底。 */
  playShutdown(done: () => void): void {
    if (this.disposed) { done(); return }
    if (this.mode === 'shutdown') return
    this.cancelMove()
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.poolTimer)
    window.clearTimeout(this.pressTimer)
    this.shutdownDone = done
    this.mode = 'shutdown'
    if (this.hasClip('shutdown', 'shutdown')) {
      void this.o.player.playOnce({ type: 'shutdown', name: 'shutdown', mood: this.state.mood })
    } else this.onAnimationIdle()
  }

  /**
   * 拆一次礼物：夹心动画只播一遍，播完回到待机（收礼的状态本身会持续几分钟，
   * 循环播「收到礼物」就是之前那个 bug）。提起时不播——落地后状态还是 gift，不会漏掉心情
   */
  playGift(foodId?: string): void {
    if (this.disposed || this.mode === 'raised') return
    if (this.mode === 'moving' || this.mode === 'move-exit') this.cancelMove()
    this.mode = 'touching'
    window.clearTimeout(this.idleTimer)
    void this.o.player.playOnce({ type: 'common', name: 'gift', mood: this.state.mood, foodId })
  }

  /** Core 只发窗口模式，具体的 Start/Loop/End 编排在 Body。 */
  handleMotion(payload: unknown): void {
    if (this.disposed || !payload || typeof payload !== 'object') return
    const event = payload as Partial<PetMotionEvent> & { side?: unknown; id?: unknown; graph?: unknown }
    if (event.kind === 'side' && (event.side === 'left' || event.side === 'right')) {
      this.enterSide(event.side)
    } else if (event.kind === 'side-stop') {
      this.leaveSide()
    } else if (event.kind === 'move' && typeof event.id === 'number' && typeof event.graph === 'string') {
      // Core 只有在当前移动碰到边界时才主动推 move（初次启动由命令返回）。
      // 若用户已开始触摸，迟到的换向事件必须反过来停掉 Core。
      if (this.mode === 'moving' && this.moveId !== null) this.enterMove(event as Extract<PetMotionEvent, { kind: 'move' }>)
      else stopPetMotion()
    } else if (event.kind === 'move-stop' && typeof event.id === 'number' && typeof event.graph === 'string') {
      this.finishMove(event.id, event.graph)
    }
  }

  /** SideHide Main ↔ Rise。只按当前帧的不透明区域算 hover，透明处不触发探头。 */
  setHovered(hovered: boolean): void {
    if (this.disposed || this.mode !== 'side' || !this.side || hovered === this.sideHovered) return
    this.sideHovered = hovered
    const side = this.side
    const generation = ++this.sideGeneration
    if (hovered) {
      const type = this.sideType(side, 'rise')
      void this.o.player.playStep({ type, name: type, mood: this.state.mood }, 'start', () =>
        this.loopSide(type, side, generation))
    } else {
      const rise = this.sideType(side, 'rise')
      void this.o.player.playStep({ type: rise, name: rise, mood: this.state.mood }, 'end', () =>
        this.loopSide(this.sideType(side, 'main'), side, generation))
    }
  }

  dispose(): void {
    this.disposed = true
    this.shutdownDone = null
    this.sideGeneration++
    this.cancelMove()
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.poolTimer)
    window.clearTimeout(this.pressTimer)
    if (this.mode === 'raised') endPetDrag()
    this.o.player.onIdle = null
    if (this.pinned) void setHitTestPinned(false)
  }

  onPointerDown(x: number, y: number, screenX = x, screenY = y): void {
    if (this.disposed || this.mode === 'raised' || this.mode === 'shutdown') return
    if (this.mode === 'side') {
      this.o.onSideExit?.()
      this.leaveSide()
    }
    this.lastAt = { x, y }
    this.pressAt = { x, y, screenX, screenY }
    this.pressOnFace = inPinchZone(this.o.profile, x, y) && this.hasClip('common', 'pinch')
    if (this.mode === 'moving' || this.mode === 'move-exit') {
      this.cancelMove()
      this.toActivity()
    }
    this.setPinned(true) // 按下期间别让穿透判定把窗口切走
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.pressTimer)
    this.pressTimer = window.setTimeout(() => {
      this.pressTimer = 0
      const at = this.lastAt
      if (this.disposed || !at) return
      this.raise()
    }, PRESS_MS)
  }

  onPointerMove(x: number, y: number, screenX = x, screenY = y): void {
    if (this.disposed) return
    this.lastAt = { x, y }
    if (this.pressAt && this.mode !== 'raised' && this.mode !== 'pinching' &&
      Math.hypot(screenX - this.pressAt.screenX, screenY - this.pressAt.screenY) >= DRAG_THRESHOLD_PX) {
      window.clearTimeout(this.pressTimer)
      this.pressTimer = 0
      // 按在脸上拖 = 捏脸（原版 Pinch）；别处拖 = 提起
      if (this.pressOnFace) this.pinch()
      else if (pressZone(this.o.profile, this.state.mood, this.pressAt.x, this.pressAt.y)) this.raise()
    }
  }

  onPointerUp(cancelled = false): void {
    if (this.disposed || !this.pressAt) return
    const wasShortPress = this.pressTimer !== 0 && !cancelled
    window.clearTimeout(this.pressTimer)
    this.pressTimer = 0
    this.pressAt = null
    if (this.mode === 'raised') {
      endPetDrag()
      this.o.onDragChange?.(false)
      return
    }
    this.setPinned(false)
    if (this.mode === 'pinching') {
      this.o.player.stop() // 松手：播 C 段放开脸，收尾回当前活动
      return
    }
    if (!wasShortPress) {
      this.scheduleIdleAction()
      return
    }

    const at = this.lastAt
    const zone = at && clickZone(this.o.profile, at.x, at.y)
    if (zone) this.touch(zone)
    else {
      this.setPinned(false)
      this.scheduleIdleAction() // 点在空白处：恢复空闲计时
    }
    this.o.onClick?.()
  }

  /** Rust 完成慢速归位或惯性落地后，才播放提起动作的收尾。 */
  onDragSettled(): void {
    if (this.disposed || this.mode !== 'raised' || this.released) return
    this.released = true
    this.setPinned(false)
  }

  /* ------------------------------------------------------------ */

  /** 只在真的变化时打一次 IPC */
  private setPinned(pinned: boolean): void {
    if (this.pinned === pinned) return
    this.pinned = pinned
    void setHitTestPinned(pinned)
  }

  /** 回到当前活动对应的循环动画。话还没说完就先接 say */
  private toActivity(): void {
    if (this.disposed || this.mode === 'shutdown') return
    if (this.moveId !== null) this.cancelMove()
    this.sideGeneration++
    this.side = null
    this.sideHovered = false
    this.mode = 'idle'
    if (!this.pressAt) this.setPinned(false)
    if (this.speaking) {
      this.playSay()
      return
    }
    // 歌正到高潮：跳舞换成 ohhhh，直到高潮过去
    if (this.climax && this.state.action?.graph === 'music' && this.hasClip('common', 'ohhhh')) {
      window.clearTimeout(this.poolTimer)
      void this.o.player.play({ type: 'common', name: 'ohhhh', mood: this.state.mood })
      return
    }
    // 工作 / 学习 / 玩：从动画池里挑一段，驻留期内被打断了回来还接着播它
    if (this.playPooled()) return
    this.pooled = null
    window.clearTimeout(this.poolTimer)
    // 这顿是麦当劳：普通动画，不用夹心（汉堡画在动画里）
    if (this.state.activity === 'eating' && this.mcdonald) {
      void this.o.player.play({ type: 'common', name: 'eatmcdonald', mood: this.state.mood })
      this.scheduleIdleAction()
      return
    }
    const { type, name } = CLIP_FOR[this.state.activity]
    // Core 指名了具体动作就用它的动画（同是 working，文案≠修屏幕）；这个类型下没有
    // 这个名字（收礼待机时 default 下没有 gift）就随机挑一个，别把不存在的目标交给播放器
    const graph = this.state.action?.graph ?? name
    void this.o.player.play({
      type,
      name: this.nameFor(type, graph && this.hasClip(type, graph) ? graph : undefined),
      mood: this.state.mood,
      foodId: this.state.action?.food?.id,
    })
    this.scheduleIdleAction()
  }

  /** 一次性状态过场：播完由统一的 onIdle 回到当前 Core 活动。 */
  private playTransition(type: GraphType, name: string): void {
    if (!this.hasClip(type, name)) {
      this.toActivity()
      return
    }
    this.mode = 'transition'
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.poolTimer)
    void this.o.player.playOnce({ type, name, mood: this.state.mood })
  }

  /** AnimationPlayer 的唯一收尾入口；按当前 Body 模式决定下一段，不改 Core 状态。 */
  private onAnimationIdle(): void {
    if (this.disposed) return
    if (this.mode === 'shutdown') {
      const done = this.shutdownDone
      this.shutdownDone = null
      done?.()
      return
    }
    if (this.mode === 'startup' && this.playSpecialDay()) return
    if (this.specialFoodId) {
      const foodId = this.specialFoodId
      this.specialFoodId = null
      if (this.hasClip('common', 'eat')) {
        this.mode = 'transition'
        void this.o.player.playOnce({
          type: 'common',
          name: 'eat',
          mood: this.state.mood,
          foodId,
        })
        return
      }
    }
    if (this.mode === 'idle-state-one' && this.state.activity === 'idle' &&
      Math.random() < 0.6 && this.hasClip('statetwo', 'state')) {
      this.mode = 'idle-state-two'
      void this.o.player.playOnce({ type: 'statetwo', name: 'state', mood: this.state.mood })
      return
    }
    this.toActivity()
  }

  /** 启动动画结束后播放当天的节日动作，并保证本次启动只触发一次。 */
  private playSpecialDay(): boolean {
    if (this.specialDayPlayed) return false
    this.specialDayPlayed = true
    const now = new Date()
    const special = SPECIAL_DAYS.find((day) => day.month === now.getMonth() + 1 && day.day === now.getDate())
    if (!special || !this.hasClip(special.type, special.name)) return false
    if (special.foodId && this.o.manifest.food.some((food) => food.id === special.foodId)) {
      this.specialFoodId = special.foodId
    }
    void this.o.player.playOnce({ type: special.type, name: special.name, mood: this.state.mood })
    this.o.onAmbientLine?.(special.text, special.speech)
    return true
  }

  /**
   * 动画池（行为树的「当前计划」）：同一个 Core 活动下按各自的驻留时长轮换表现动画。
   * 返回 false = 这个活动不在池里，走普通映射
   */
  private playPooled(): boolean {
    const activity = this.state.activity
    // Core 指名的动画自己有池（跳舞）就用它的，否则按活动
    const graph = this.state.action?.graph
    const pool = (graph && ACTION_POOLS[graph]) || ANIMATION_POOLS[activity]
    if (!pool) return false
    const now = Date.now()
    const usable = (e: PoolEntry) => this.hasClip(e.type, e.name)
    let current = this.pooled
    if (!current || current.activity !== activity || now >= current.until) {
      // 刚进这个活动优先用 Core 指名的动画；驻留期到了就换一个不同的
      const prefer = current?.activity === activity ? undefined : this.state.action?.graph
      const entry = pickEntry(pool, usable, prefer, current?.entry.name)
      if (!entry) return false
      current = { activity, entry, until: now + dwellMs(entry) }
      this.pooled = current
    }
    void this.o.player.play({ type: current.entry.type, name: current.entry.name, mood: this.state.mood })
    window.clearTimeout(this.poolTimer)
    this.poolTimer = window.setTimeout(() => {
      // 驻留期到：只有真的闲着（没在摸、没在说、没被提起）才换；否则等那段交互结束后
      // toActivity 会看到 until 过了，自然换
      if (!this.disposed && this.mode === 'idle' && this.state.activity === activity) this.toActivity()
    }, Math.max(1000, current.until - now))
    window.clearTimeout(this.idleTimer)
    return true
  }

  /** 体力、心情、饱腹、口渴、健康、好感全在高位 */
  private feelingGreat(): boolean {
    const s = this.state
    return s.strength >= KISS_MIN.strength && s.feeling >= KISS_MIN.feeling && s.hunger >= KISS_MIN.hunger
      && s.thirst >= KISS_MIN.thirst && s.health >= KISS_MIN.health && s.affection >= KISS_MIN.affection
  }

  /** (type, name) 在资源里到底有没有动画（夹心的或普通的任一段） */
  private hasClip(type: GraphType, name: string): boolean {
    const m = this.o.manifest
    const mood = this.state.mood
    return !!resolveLayered(m, type, name, mood)
      || (['start', 'loop', 'single', 'end'] as const).some((a) => resolveClips(m, type, name, mood, a).length > 0)
  }

  /** 没指定名字时按心情随机挑一个（原版 GraphCore.FindName 的语义） */
  private nameFor(type: GraphType, explicit?: string): string | undefined {
    if (explicit) return explicit
    const names = namesFor(this.o.manifest, type, this.state.mood)
    return names.length ? pick(names) : undefined
  }

  /** 只有真正空闲时才插小动作——工作/睡觉时乱插会打断那个活动的循环 */
  private scheduleIdleAction(): void {
    window.clearTimeout(this.idleTimer)
    if (this.state.activity !== 'idle') return
    const [lo, hi] = IDLE_ACTION_EVERY_MS
    this.idleTimer = window.setTimeout(
      () => void this.runIdleAction(),
      lo + Math.random() * (hi - lo),
    )
  }

  private async runIdleAction(): Promise<void> {
    if (this.disposed || this.mode !== 'idle' || this.state.activity !== 'idle') return
    const requestGeneration = this.moveGeneration
    if (Math.random() < MOVE_CHANCE) {
      const event = await startPetMotion(this.state.mood)
      if (this.disposed || this.mode !== 'idle' || this.state.activity !== 'idle' || requestGeneration !== this.moveGeneration) {
        if (event?.kind === 'move') stopPetMotion()
        return
      }
      if (event?.kind === 'move') {
        this.enterMove(event)
        return
      }
    }

    // 庆祝动作单独抽样，避免和 MI / MU 共用一个三选一概率导致 BDay 很难出现。
    if (Math.random() < CELEBRATION_CHANCE && this.hasClip('common', 'bday')) {
      void this.o.player.playOnce({ type: 'common', name: 'bday', mood: this.state.mood })
      this.sayLoveLine('今天也要开心呀，喜欢你。', {
        emotion: 'happy', energy: 0.78, style: 'playful',
      })
      return
    }
    // 全身状态都很好的时候，偶尔来个飞吻（WORK/kiss）
    if (this.feelingGreat() && Math.random() < KISS_CHANCE && this.hasClip('work', 'kiss')) {
      void this.o.player.playOnce({ type: 'work', name: 'kiss', mood: this.state.mood })
      this.sayLoveLine('给你一个飞吻，接住哦。', {
        emotion: 'shy', energy: 0.68, style: 'warm',
      })
      return
    }

    // 日常：伸懒腰 / 喝茶（Relax 的 MI、MU）、过生日那套（BDay）
    if (Math.random() < RELAX_CHANCE) {
      const names = RELAX_NAMES.filter((n) => this.hasClip('common', n))
      if (names.length) {
        const [lo, hi] = RELAX_PLAY_MS
        void this.o.player.playFor(
          { type: 'common', name: pick(names), mood: this.state.mood },
          lo + Math.random() * (hi - lo),
        )
        return
      }
    }
    // 原版的 StateONE / StateTWO 是同一套待机姿态的两阶段变化，不属于 Core 活动。
    if (Math.random() < STATE_IDLE_CHANCE && this.hasClip('stateone', 'state')) {
      this.mode = 'idle-state-one'
      void this.o.player.playOnce({ type: 'stateone', name: 'state', mood: this.state.mood })
      return
    }

    const names = namesFor(this.o.manifest, 'idel', this.state.mood)
    if (names.length) void this.o.player.playOnce({ type: 'idel', name: pick(names), mood: this.state.mood })
    else this.scheduleIdleAction()
  }

  private sayLoveLine(text: string, speech: SpeechCue): void {
    const now = Date.now()
    if (this.state.mood !== 'happy'
      || now - this.lastLoveLineAt < LOVE_LINE_COOLDOWN_MS
      || text === this.lastLoveLineText) return
    this.lastLoveLineAt = now
    this.lastLoveLineText = text
    this.o.onAmbientLine?.(text, speech)
  }

  /** Core 听声音：歌到高潮 / 过去了。只在她正跟着歌跳的时候换画面 */
  setClimax(on: boolean): void {
    if (this.disposed || this.climax === on) return
    this.climax = on
    if (this.mode !== 'idle' || this.state.action?.graph !== 'music') return
    if (on && this.hasClip('common', 'ohhhh')) {
      window.clearTimeout(this.poolTimer)
      void this.o.player.play({ type: 'common', name: 'ohhhh', mood: this.state.mood })
    } else {
      this.toActivity() // 回到池里原来那段（驻留没到不重抽）
    }
  }

  /** 捏脸：A 段捏住 → B 段循环到松手 → C 段放开。窗口不动，算一次摸头 */
  private pinch(): void {
    if (this.mode === 'pinching') return
    this.mode = 'pinching'
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.poolTimer)
    void this.o.player.play({ type: 'common', name: 'pinch', mood: this.state.mood })
    this.o.onTouch?.('head')
  }

  private touch(zone: ClickZone): void {
    this.mode = 'touching'
    const mood = this.state.mood
    const type = zone === 'head' ? 'touch_head' : 'touch_body'
    void this.o.player.playOnce({ type, name: this.nameFor(type), mood })
    this.o.onTouch?.(zone)
  }

  private raise(): void {
    const anchor = this.pressAt
    if (!anchor || this.mode === 'raised' ||
      !pressZone(this.o.profile, this.state.mood, anchor.x, anchor.y)) return
    this.mode = 'raised'
    this.released = false
    this.struggles = 0
    const mood = this.state.mood
    const names = namesFor(this.o.manifest, 'raised_static', mood)
    this.raiseName = names.length ? pick(names) : 'raise'
    this.raiseVariant = raiseVariant(this.o.profile, mood, anchor.x)
    this.o.onTouch?.('raise')
    this.o.onDragChange?.(true)
    // Keep the original grab point. Rust follows the physical cursor directly,
    // avoiding asynchronous relative-position races and jumps to the head.
    beginPetDrag(anchor.x, anchor.y)
    this.raiseStep()
  }

  /** 原版 MainDisplay.DisplayRaising 的 rasetype 递归 */
  private raiseStep = (): void => {
    // 放下时 Core 可能已经判定越过屏幕边缘并切进 SideHide。旧的异步解码
    // 回调不能再播落地 / toActivity，把刚开始的侧挂动画抢回去。
    if (this.disposed || this.mode !== 'raised') return
    const mood = this.state.mood
    const name = this.raiseName
    if (this.released) {
      void this.o.player.playStep({ type: 'raised_static', name, mood, variant: this.raiseVariant }, 'end', () => this.toActivity())
      return
    }
    if (this.struggles < STRUGGLE_TIMES) {
      this.struggles++
      void this.o.player.playStep({ type: 'raised_dynamic', name, mood, variant: this.raiseVariant }, 'single', this.raiseStep)
      return
    }
    if (this.struggles === STRUGGLE_TIMES) {
      this.struggles++
      void this.o.player.playStep({ type: 'raised_static', name, mood, variant: this.raiseVariant }, 'start', this.raiseStep)
      return
    }
    void this.o.player.playStep({ type: 'raised_static', name, mood, variant: this.raiseVariant }, 'loop', this.raiseStep)
  }

  /* ------------------------ 自主移动 ------------------------ */

  private enterMove(event: Extract<PetMotionEvent, { kind: 'move' }>): void {
    if (this.disposed || (this.mode !== 'idle' && this.mode !== 'moving')) {
      stopPetMotion()
      return
    }
    window.clearTimeout(this.idleTimer)
    this.mode = 'moving'
    this.moveId = event.id
    this.moveGraph = event.graph
    const generation = ++this.moveGeneration
    void this.o.player.playStep(
      { type: 'move', name: event.graph, mood: this.state.mood },
      'start',
      () => void this.beginMoveLoop(event.id, event.graph, generation),
    )
  }

  private async beginMoveLoop(id: number, graph: string, generation: number): Promise<void> {
    if (!this.isCurrentMove(id, graph, generation)) return
    const began = await beginPetMotionStep(id)
    if (!this.isCurrentMove(id, graph, generation)) return
    if (began !== true) {
      this.cancelMove()
      this.toActivity()
      return
    }
    this.loopMove(id, graph, generation)
  }

  private loopMove(id: number, graph: string, generation: number): void {
    if (!this.isCurrentMove(id, graph, generation)) return
    void this.o.player.playStep(
      { type: 'move', name: graph, mood: this.state.mood },
      'loop',
      () => void this.completeMoveLoop(id, graph, generation),
    )
  }

  private async completeMoveLoop(id: number, graph: string, generation: number): Promise<void> {
    if (!this.isCurrentMove(id, graph, generation)) return
    const decision = await completePetMotionCycle(id)
    if (!this.isCurrentMove(id, graph, generation)) return
    if (decision?.kind === 'move-continue' && decision.id === id) {
      this.loopMove(id, graph, generation)
    } else if (decision?.kind === 'move') {
      // 原版切兼容动作时直接播新 A_Start，不播旧 C_End。
      this.enterMove(decision)
    } else if (decision?.kind === 'move-stop' && decision.id === id) {
      this.finishMove(id, decision.graph)
    } else {
      this.cancelMove()
      this.toActivity()
    }
  }

  private finishMove(id: number, graph: string): void {
    if (this.disposed || this.mode !== 'moving' || this.moveId !== id) return
    this.moveId = null
    this.moveGraph = null
    this.mode = 'move-exit'
    const generation = ++this.moveGeneration
    void this.o.player.playStep({ type: 'move', name: graph, mood: this.state.mood }, 'end', () => {
      if (!this.disposed && this.mode === 'move-exit' && generation === this.moveGeneration) this.toActivity()
    })
  }

  private isCurrentMove(id: number, graph: string, generation: number): boolean {
    return !this.disposed && this.mode === 'moving' && this.moveId === id &&
      this.moveGraph === graph && this.moveGeneration === generation
  }

  private cancelMove(): void {
    if (this.moveId !== null) stopPetMotion()
    this.moveId = null
    this.moveGraph = null
    this.moveGeneration++
    if (this.mode === 'moving' || this.mode === 'move-exit') this.mode = 'idle'
  }

  /* ------------------------ 左右侧挂 ------------------------ */

  private sideType(side: 'left' | 'right', phase: 'main' | 'rise'): GraphType {
    return `sidehide_${side}_${phase}` as GraphType
  }

  private enterSide(side: 'left' | 'right'): void {
    this.cancelMove()
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.pressTimer)
    this.side = side
    this.sideHovered = false
    this.mode = 'side'
    const generation = ++this.sideGeneration
    const type = this.sideType(side, 'main')
    void this.o.player.playStep({ type, name: type, mood: this.state.mood }, 'start', () =>
      this.loopSide(type, side, generation))
  }

  private loopSide(type: GraphType, side: 'left' | 'right', generation: number): void {
    if (this.disposed || generation !== this.sideGeneration || this.mode !== 'side' || this.side !== side) return
    void this.o.player.playStep({ type, name: type, mood: this.state.mood }, 'loop', () =>
      this.loopSide(type, side, generation))
  }

  private leaveSide(): void {
    const side = this.side
    if (!side || !['side', 'side-exit'].includes(this.mode)) return
    this.side = null
    this.sideHovered = false
    this.mode = 'side-exit'
    const generation = ++this.sideGeneration
    const type = this.sideType(side, 'main')
    void this.o.player.playStep({ type, name: type, mood: this.state.mood }, 'end', () => {
      if (!this.disposed && generation === this.sideGeneration && this.mode === 'side-exit') this.toActivity()
    })
  }

}
