import { invokeCore } from './ipc'

export type PetMotionEvent =
  | { kind: 'side'; side: 'left' | 'right' }
  | { kind: 'side-stop' }
  | { kind: 'move'; id: number; graph: string }
  | { kind: 'move-continue'; id: number }
  | { kind: 'move-stop'; id: number; graph: string }

/** 把当前帧的命中掩码推给 Rust 的穿透判定（按位打包的 48×48） */
export const pushHitMask = (cells: Uint8Array) => void invokeCore('set_hit_mask', { cells: Array.from(cells) })

/**
 * 交互期间钉住窗口不穿透。不钉的话，提起后把宠物拖到光标不再压着它的位置时，
 * 轮询会把窗口切成穿透，拖拽当场断掉。
 */
export const setHitTestPinned = (pinned: boolean) => void invokeCore('set_hit_test_pinned', { pinned })

/** 报告宠物被摸了。数值怎么变是 Core 状态机的事，Body 不自己算 */
export const reportTouch = (zone: string) => void invokeCore('pet_touched', { zone })

/**
 * 把食物目录交给 Core。食物数据来自 manifest（build-assets 从原版转出的 123 项），
 * 但「买哪样」是状态机的决定——它得先看得见这张表。
 */
export const pushFoodCatalog = (items: unknown[]) => void invokeCore('set_food_catalog', { items })

/**
 * 把 pet.json 里的原版移动规则和贴边锚点交给 Rust。窗口几何必须在 Core 里算；
 * Body 只负责按照 `pet:motion` 事件播放对应动画。
 */
export const pushMotionProfile = (profile: { moves: unknown[]; side: unknown }) =>
  void invokeCore('set_pet_motion_profile', { profile })

/** 侧挂状态下先把窗口完整拉回屏幕，再交给普通点击 / 提起逻辑。 */
export const exitPetSide = () => void invokeCore('exit_pet_side')

/** 真正空闲时请 Core 按原版 Trigger / Mode 规则挑一个可用移动。 */
export const startPetMotion = (mood: string) => invokeCore<PetMotionEvent>('start_pet_motion', { mood })

/** start 段结束后，Core 才应用 Locate 并按原版 125 ms 步进窗口。 */
export const beginPetMotionStep = (id: number) => invokeCore<boolean>('begin_pet_motion_step', { id })

/** 一轮 B_Loop 结束后执行原版 Distance / 兼容动作判定。 */
export const completePetMotionCycle = (id: number) =>
  invokeCore<PetMotionEvent>('complete_pet_motion_cycle', { id })

/** 触摸、说话、状态变化都优先于自主移动。 */
export const stopPetMotion = () => void invokeCore('stop_pet_motion')

/** 送她一样礼物（随机挑一件）。返回礼物名字，没货架时返回 null */
export const giveGift = (id?: string) => invokeCore<string>('give_gift', { id })

/**
 * 按逻辑像素平移宠物窗口——提起时用来让窗口跟住光标。
 * 浏览器预览里没有 Tauri，静默跳过（动画照常，只是窗口不动）。
 */
// Serialize begin/end so a quick release cannot overtake an async begin IPC.
let dragQueue: Promise<unknown> = Promise.resolve()
export function beginPetDrag(anchorX: number, anchorY: number): void {
  dragQueue = dragQueue.then(() => invokeCore('begin_pet_drag', { anchorX, anchorY }))
}
export function endPetDrag(): void {
  dragQueue = dragQueue.then(() => invokeCore('end_pet_drag'))
}

export const openChat = () => invokeCore('open_chat')
export const openSettingsPanel = () => invokeCore('open_settings_panel')
