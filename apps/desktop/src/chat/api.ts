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
  speed: number
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
}
export interface ActionInfo { id: string; name: string; defaultLines: string[] }
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
export interface StreamEvent { requestId: string; delta: string; done: boolean; text?: string }

export const DEFAULT_VOICE_SETTINGS: VoiceSettings = {
  enabled: true, endpoint: 'http://127.0.0.1:8090', voice: 'serena', style: '', moodStyle: true, speed: 1,
  speakChat: true, speakLines: true,
}
export const DEFAULT_LINE_SETTINGS: LineSettings = { enabled: true, mode: 'fixed', minGapSec: 45, actions: {} }

export const DEFAULT_CHAT_SETTINGS: ChatSettings = {
  model: 'qwen3.5:9b', endpoint: 'http://127.0.0.1:11434', temperature: 0.75,
  voice: DEFAULT_VOICE_SETTINGS, lines: DEFAULT_LINE_SETTINGS,
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
