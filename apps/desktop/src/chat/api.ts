import { IS_TAURI } from '../body/ipc'

export interface Persona {
  name: string
  background: string
  appearance: string
  personality: string
  speakingStyle: string
}
/** 与 src-tauri/src/tts.rs 的 VoiceSettings 对应 */
export interface VoiceSettings {
  enabled: boolean
  endpoint: string
  voice: string
  style: string
  moodStyle: boolean
  /** 播放倍速（不保持音高：快一点就高一点） */
  speed: number
  keepPitch: boolean
  speakChat: boolean
  speakLines: boolean
}
export type LineMode = 'fixed' | 'model' | 'off'
/** 与 src-tauri/src/lines.rs 的 LineSettings 对应 */
export interface LineSettings {
  enabled: boolean
  mode: LineMode
  minGapSec: number
  /** 按动作 id 覆盖；lines 为 null = 用动作表里的默认台词 */
  actions: Record<string, { mode: LineMode | null; lines: string[] | null }>
}
export interface ChatSettings {
  model: string
  endpoint: string
  temperature: number
  persona: Persona
  voice: VoiceSettings
  lines: LineSettings
  /** 按知识库日程的主动提醒（nudge.rs）。面板暂时不改它，但保存设置时要原样带回去 */
  nudges: NudgeSettings
}
export interface NudgeSettings {
  enabled: boolean
  /** 开始前多少分钟提醒 */
  leadMin: number
  /** 多久没动键鼠算不在（秒） */
  idleMaxSec: number
  /** 一天主动搭话几次（自言自语不算） */
  dailyBudget: number
  /** 让模型隔一阵子嘀咕一句 */
  selfTalk: boolean
  /** 自言自语 / 关心你歇一下的音量 0–1 */
  quietVolume: number
}
export interface ActionInfo { id: string; name: string; defaultLines: string[] }
/** 与 core/scheduler.rs 的 Timer 一致；focus 非空 = 专注段，头顶有倒计时 */
export interface TimerInfo { id: string; label: string; dueAt: number; repeatMs: number | null; focus?: { startedAt: number; target: string | null } | null }
export interface TtsStatus { connected: boolean; endpoint: string; voices: string[]; error: string | null }
export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  createdAt: number
  status: 'complete' | 'cancelled' | 'error'
  rating: 'up' | 'down' | null
  correctedText: string | null
  /** intent = 使唤的判定台词（模型没回上话时的兜底） */
  source: 'model' | 'memory' | 'intent'
}
export interface ModelStatus { connected: boolean; models: string[]; error: string | null }
export interface DesktopSettings { size: number; alwaysOnTop: boolean }
/** reset = 模型没进角色重来了一次，前面流出来的字作废 */
export interface StreamEvent { requestId: string; delta: string; done: boolean; text?: string; reset?: boolean }

/** 与 tts.rs 的 NEURO_STYLE / VoiceSettings::default 一致 */
export const NEURO_STYLE = '语气平稳、起伏小，节奏偏快，音调偏高，像轻快的电子少女音 / flat calm intonation, quick pace, slightly high pitch, light synthetic girl voice'
export const DEFAULT_VOICE_SETTINGS: VoiceSettings = {
  enabled: true, endpoint: 'http://127.0.0.1:8090', voice: 'vivian', style: NEURO_STYLE, moodStyle: false, speed: 1.12,
  keepPitch: false, speakChat: true, speakLines: true,
}
export const DEFAULT_LINE_SETTINGS: LineSettings = { enabled: true, mode: 'fixed', minGapSec: 45, actions: {} }
/** 与 nudge.rs 的 NudgeSettings::default 成对改 */
export const DEFAULT_NUDGE_SETTINGS: NudgeSettings = { enabled: true, leadMin: 15, idleMaxSec: 300, dailyBudget: 6, selfTalk: true, quietVolume: 0.5 }

export const DEFAULT_CHAT_SETTINGS: ChatSettings = {
  model: 'qwen3.5:9b', endpoint: 'http://127.0.0.1:11434', temperature: 0.75,
  voice: DEFAULT_VOICE_SETTINGS, lines: DEFAULT_LINE_SETTINGS, nudges: DEFAULT_NUDGE_SETTINGS,
  // 与 chat.rs 的 Persona::default 保持一致；真正生效的是 Core 里的那份，这里只是加载前的占位
  persona: {
    name: '萝莉斯',
    background: '住在用户桌面上的伙伴，陪伴日常生活、学习和工作，有自己的喜好与小脾气。',
    appearance: '与桌面立绘一致：银灰色长发、头顶一撮呆毛，金黄色的眼睛，穿着舒适可爱的日常装扮。',
    personality: '温柔、好奇、坦率，亲近但有边界。认真倾听，不一味附和。',
    speakingStyle: '用自然的简体中文聊天，通常两到四句话。像熟悉的朋友，不用客服套话；偶尔有轻巧的动作描写，不每句都撒娇。',
  },
}

/** Mutations must surface Core errors; a browser preview never pretends it saved. */
export async function invokeStrict<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!IS_TAURI) throw new Error('当前是浏览器预览，请在桌宠应用中使用这个功能。')
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(command, args)
}
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
