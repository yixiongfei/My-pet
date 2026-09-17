import { z } from 'zod'
import { Mood } from './pet'

/**
 * pet.json 里所有坐标的参考系：原版把宠物画在 500×500 的逻辑画布上
 * （vup.lps 的 touchraised sw=500 就是整幅宽度）。与 manifest.size 无关——
 * 出高清资产时 manifest.size 变了，这个参考系不变。
 */
export const PET_LOGICAL_SIZE = 500

export const Rect = z.object({ px: z.number(), py: z.number(), sw: z.number(), sh: z.number() })
export type Rect = z.infer<typeof Rect>

export interface Point {
  x: number
  y: number
}

export const LocateType = z.enum(['None', 'Left', 'Right', 'Top', 'Bottom'])
export type LocateType = z.infer<typeof LocateType>

/**
 * 原版 vup.lps 的一条移动规则。方向位与 GraphHelper.Move.DirectionType 一致：
 * Left/Right/Top/Bottom = 1/2/4/8，带 Greater 的反向边界 = 16/32/64/128。
 */
export interface MoveRule {
  graph: string
  locateType: LocateType
  locateLength: number
  interval: number
  checkType: number
  checkLeft: number
  checkRight: number
  checkTop: number
  checkBottom: number
  modeType: number
  triggerType: number
  triggerLeft: number
  triggerRight: number
  triggerTop: number
  triggerBottom: number
  speedX: number
  speedY: number
  distance: number
}

export interface SideAnchors {
  left: number
  right: number
}

const MoveJson = z.object({
  graph: z.string().min(1),
  LocateType: LocateType.default('None'),
  LocateLength: z.number().default(0),
  Interval: z.number().int().positive().default(125),
  CheckType: z.number().int().default(0),
  CheckLeft: z.number().default(100),
  CheckRight: z.number().default(100),
  CheckTop: z.number().default(100),
  CheckBottom: z.number().default(100),
  ModeType: z.number().int().default(30),
  TriggerType: z.number().int().default(0),
  TriggerLeft: z.number().default(100),
  TriggerRight: z.number().default(100),
  TriggerTop: z.number().default(100),
  TriggerBottom: z.number().default(100),
  SpeedX: z.number().default(0),
  SpeedY: z.number().default(0),
  Distance: z.number().positive().default(5),
})

/** build-assets 从 vup.lps 转出的 pet.json，只声明 Body 用得到的字段 */
const PetJson = z.object({
  pet: z.object({ petname: z.string().optional() }),
  touchhead: Rect,
  touchbody: Rect,
  /** lps 把按心情分的字段平铺成 happy_px / nomal_px / …，见 byMood */
  touchraised: z.record(z.string(), z.number()),
  raisepoint: z.record(z.string(), z.number()),
  move: z.array(MoveJson).default([]),
  side: z.object({ left: z.number(), right: z.number() }),
})

/** 归一化后的宠物配置：按心情平铺的字段收回成 Record<Mood, …> */
export interface PetProfile {
  name: string
  /** 短按摸头的命中区域 */
  touchHead: Rect
  /** 短按摸身体的命中区域 */
  touchBody: Rect
  /** 长按提起的命中区域；ill 时宠物瘫着，区域整体下移 */
  touchRaised: Record<Mood, Rect>
  /** 提起时贴合光标的锚点（宠物被拎住的那个点） */
  raisePoint: Record<Mood, Point>
  /** 原版的走路、爬行、爬墙和坠落规则 */
  moves: MoveRule[]
  /** 左右侧挂时，500×500 身体画布中留在屏幕里的锚点 */
  side: SideAnchors
}

export function parsePetProfile(json: unknown): PetProfile {
  const raw = PetJson.parse(json)
  return {
    name: raw.pet.petname ?? 'vup',
    touchHead: raw.touchhead,
    touchBody: raw.touchbody,
    touchRaised: byMood(raw.touchraised, 'touchraised', (v) => ({ px: v('px'), py: v('py'), sw: v('sw'), sh: v('sh') })),
    raisePoint: byMood(raw.raisepoint, 'raisepoint', (v) => ({ x: v('x'), y: v('y') })),
    moves: raw.move.map((m) => ({
      graph: m.graph,
      locateType: m.LocateType,
      locateLength: m.LocateLength,
      interval: m.Interval,
      checkType: m.CheckType,
      checkLeft: m.CheckLeft,
      checkRight: m.CheckRight,
      checkTop: m.CheckTop,
      checkBottom: m.CheckBottom,
      modeType: m.ModeType,
      triggerType: m.TriggerType,
      triggerLeft: m.TriggerLeft,
      triggerRight: m.TriggerRight,
      triggerTop: m.TriggerTop,
      triggerBottom: m.TriggerBottom,
      speedX: m.SpeedX,
      speedY: m.SpeedY,
      distance: m.Distance,
    })),
    side: raw.side,
  }
}

function byMood<T>(
  flat: Record<string, number>,
  field: string,
  build: (v: (suffix: string) => number) => T,
): Record<Mood, T> {
  const out = {} as Record<Mood, T>
  for (const mood of Mood.options) {
    out[mood] = build((suffix) => {
      const v = flat[`${mood}_${suffix}`]
      if (typeof v !== 'number') throw new Error(`pet.json ${field} 缺少 ${mood}_${suffix}`)
      return v
    })
  }
  return out
}
