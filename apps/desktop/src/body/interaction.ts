import type { GraphType, Manifest, PetProfile, PetState } from '@vpet/shared'
import type { AnimationPlayer } from './AnimationPlayer'
import { ANIMATION_POOLS, dwellMs, pickEntry, type PoolEntry } from './animationPool'
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
import { clickZone, type ClickZone } from './touch'

/** 按住多久算长按（原版 Setting.PressLength，默认 0.5s） */
const PRESS_MS = 500
const DRAG_THRESHOLD_PX = 6
/** 空闲多久随机播一个小动作 */
const IDLE_ACTION_EVERY_MS: [number, number] = [15_000, 40_000]
/** 原版每次 15 秒空闲判定约有 3/20 机会移动；这里计时更稀疏，适当提高单次命中。 */
const MOVE_CHANCE = 0.4
/** 没去移动时，进入原版 StateONE → StateTWO 成对待机姿态的概率。 */
const STATE_IDLE_CHANCE = 0.35
/** 提起后先挣扎几次再转静止（原版 rasetype 0→2，共 3 次 Raised_Dynamic） */
const STRUGGLE_TIMES = 3
const MOOD_RANK: Record<PetState['mood'], number> = { ill: 0, poorcondition: 1, nomal: 2, happy: 3 }

type InteractionMode =
  | 'idle' | 'touching' | 'raised' | 'talking' | 'moving' | 'move-exit' | 'side' | 'side-exit'
  | 'startup' | 'shutdown' | 'thinking' | 'transition' | 'idle-state-one' | 'idle-state-two'

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
  private pinned = false
  private disposed = false
  /** 正在出声（语音在放）。一次性动画播完后 toActivity 会先接上 say 动画 */
  private speaking = false
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

  constructor(private readonly o: InteractionOpts) {}

  /** 开始播当前活动对应的动画 + 空闲小动作循环 */
  start(): void {
    this.o.player.onIdle = () => this.onAnimationIdle()
    if (this.hasClip('startup', 'startup')) {
      this.mode = 'startup'
      void this.o.player.playOnce({ type: 'startup', name: 'startup', mood: this.state.mood })
    } else this.toActivity()
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
  startSay(): void {
    if (this.disposed) return
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
    void this.o.player.play({ type: 'say', name: this.nameFor('say'), mood: this.state.mood })
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
      void this.o.player.playStep({ type, mood: this.state.mood }, 'start', () =>
        this.loopSide(type, side, generation))
    } else {
      const rise = this.sideType(side, 'rise')
      void this.o.player.playStep({ type: rise, mood: this.state.mood }, 'end', () =>
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
    endPetDrag()
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
    if (this.pressAt && this.mode !== 'raised' &&
      Math.hypot(screenX - this.pressAt.screenX, screenY - this.pressAt.screenY) >= DRAG_THRESHOLD_PX) {
      window.clearTimeout(this.pressTimer)
      this.pressTimer = 0
      this.raise()
    }
  }

  onPointerUp(cancelled = false): void {
    if (this.disposed || !this.pressAt) return
    const wasShortPress = this.pressTimer !== 0 && !cancelled
    window.clearTimeout(this.pressTimer)
    this.pressTimer = 0
    this.pressAt = null
    endPetDrag()
    this.setPinned(false)

    if (this.mode === 'raised') {
      this.released = true // 等当前段播完再落地，落地后 toActivity 解钉
      this.o.onDragChange?.(false)
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
    // 工作 / 学习 / 玩：从动画池里挑一段，驻留期内被打断了回来还接着播它
    if (this.playPooled()) return
    this.pooled = null
    window.clearTimeout(this.poolTimer)
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
    if (this.mode === 'idle-state-one' && this.state.activity === 'idle' &&
      Math.random() < 0.6 && this.hasClip('statetwo', 'state')) {
      this.mode = 'idle-state-two'
      void this.o.player.playOnce({ type: 'statetwo', name: 'state', mood: this.state.mood })
      return
    }
    this.toActivity()
  }

  /**
   * 动画池（行为树的「当前计划」）：同一个 Core 活动下按各自的驻留时长轮换表现动画。
   * 返回 false = 这个活动不在池里，走普通映射
   */
  private playPooled(): boolean {
    const activity = this.state.activity
    const pool = ANIMATION_POOLS[activity]
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

  private touch(zone: ClickZone): void {
    this.mode = 'touching'
    const mood = this.state.mood
    const type = zone === 'head' ? 'touch_head' : 'touch_body'
    void this.o.player.playOnce({ type, name: this.nameFor(type), mood })
    this.o.onTouch?.(zone)
  }

  private raise(): void {
    const anchor = this.pressAt
    if (!anchor || this.mode === 'raised') return
    this.mode = 'raised'
    this.released = false
    this.struggles = 0
    const mood = this.state.mood
    const names = namesFor(this.o.manifest, 'raised_static', mood)
    this.raiseName = names.length ? pick(names) : 'raise'
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
      void this.o.player.playStep({ type: 'raised_static', name, mood }, 'end', () => this.toActivity())
      return
    }
    if (this.struggles < STRUGGLE_TIMES) {
      this.struggles++
      void this.o.player.playStep({ type: 'raised_dynamic', name, mood }, 'single', this.raiseStep)
      return
    }
    if (this.struggles === STRUGGLE_TIMES) {
      this.struggles++
      void this.o.player.playStep({ type: 'raised_static', name, mood }, 'start', this.raiseStep)
      return
    }
    void this.o.player.playStep({ type: 'raised_static', name, mood }, 'loop', this.raiseStep)
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
    void this.o.player.playStep({ type, mood: this.state.mood }, 'start', () =>
      this.loopSide(type, side, generation))
  }

  private loopSide(type: GraphType, side: 'left' | 'right', generation: number): void {
    if (this.disposed || generation !== this.sideGeneration || this.mode !== 'side' || this.side !== side) return
    void this.o.player.playStep({ type, mood: this.state.mood }, 'loop', () =>
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
    void this.o.player.playStep({ type, mood: this.state.mood }, 'end', () => {
      if (!this.disposed && generation === this.sideGeneration && this.mode === 'side-exit') this.toActivity()
    })
  }

}
