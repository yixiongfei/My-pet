import type { Activity, GraphType } from '@vpet/shared'

/**
 * 行为树的「当前计划」一层：每个 Core 活动下有一池表现动画，各自带最短 / 最长驻留时间。
 *
 *   Work  → 写文案 8–15 min · 清屏 5–10 · 直播 10–20 · 烧烤 6–12 · 修屏幕 6–12
 *   Study → 看书 · 阅读 · 写字 · 研究 · 画画
 *   Play  → 打游戏 · 删错误 · 跳绳 · 玩水 · 打网球 · 两套舞蹈
 *
 * Core 的活动（Work/Study/…）、时长和数值一概不变；这里只管画面：驻留期没到不因为
 * 轮换而换动画，到了就在同一池里随机换一个不同的。用户交互、生理急需、活动自然结束
 * 照样随时打断——那些走 Interaction 原有的路。睡觉 / 发呆 / 吃喝 / 收礼不在池里。
 */
export interface PoolEntry {
  type: GraphType
  name: string
  minMin: number
  maxMin: number
}

const work = (name: string, minMin: number, maxMin: number): PoolEntry => ({ type: 'work', name, minMin, maxMin })
const common = (name: string, minMin: number, maxMin: number): PoolEntry => ({ type: 'common', name, minMin, maxMin })

export const ANIMATION_POOLS: Partial<Record<Activity, PoolEntry[]>> = {
  working: [
    work('workone', 8, 15),        // 写文案
    work('workclean', 5, 10),      // 清屏
    work('worktwo', 10, 20),       // 直播
    work('grilledsausage', 6, 12), // 烧烤
    work('fixmenu', 6, 12),        // 修屏幕
  ],
  studying: [
    work('study', 8, 15),          // 看书
    work('reading', 8, 15),        // 阅读（WORK/reading，只有 nomal 时由 mood fallback 复用）
    work('calligraphy', 6, 12),    // 写字
    work('studytwo', 6, 12),       // 研究 / 思考
    work('studypaint', 8, 15),     // 画画
  ],
  playing: [
    work('playone', 6, 12),        // 打游戏
    work('removeobject', 5, 10),   // 删错误
    work('ropeskipping', 3, 6),    // 跳绳
    work('playwater', 5, 10),      // 玩水
    common('tennis', 4, 8),        // 打网球（原 Relax 资源）
    common('music', 4, 8),         // 舞蹈 1
    common('music2', 4, 8),        // 舞蹈 2
  ],
}

/**
 * 按 Core 指名的动画（`action.graph`）单独开的池，优先于按活动的池：
 * 「跟着歌跳舞」是 playing，但只该在舞蹈动画里轮换，不该跳着跳着去打游戏
 */
export const ACTION_POOLS: Record<string, PoolEntry[]> = {
  music: [
    common('music', 3, 6),         // 跳舞 1（Music）
    common('music2', 2, 4),        // 跳舞 2（Music2）
    common('cosplay', 2, 4),       // 换装（saraburate）
    common('ohhhh', 1, 3),         // 嗨起来（saraburate）
  ],
}

/** 抽一个驻留时长（毫秒） */
export const dwellMs = (e: PoolEntry): number => (e.minMin + Math.random() * (e.maxMin - e.minMin)) * 60_000

/**
 * 从池里挑下一段：`exclude` 是刚播完的那个，能换就不重复；`prefer` 是 Core 指名的
 * 动画（同是 working，文案≠修屏幕），刚进这个活动时优先用它
 */
export function pickEntry(pool: PoolEntry[], usable: (e: PoolEntry) => boolean, prefer?: string, exclude?: string): PoolEntry | undefined {
  const candidates = pool.filter(usable)
  if (!candidates.length) return undefined
  if (prefer) {
    const hit = candidates.find((e) => e.name === prefer)
    if (hit) return hit
  }
  const others = candidates.filter((e) => e.name !== exclude)
  const from = others.length ? others : candidates
  return from[Math.floor(Math.random() * from.length)]
}
