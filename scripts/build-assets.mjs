#!/usr/bin/env node
/**
 * build-assets.mjs —— 把 assets-src/pet/<pet>/ 下的原版 PNG 帧动画转成前端可用的资产。
 *
 *   assets-src/pet/vup.json            →  动画与角色配置（唯一运行时数据源）
 *   assets-src/pet/vup/**              →  apps/desktop/public/pet/<clipId>/<n>.webp
 *   assets-src/pet/vup.lps + info.lps  →  只由 --convert-pet 一次性迁移到 vup.json
 *   assets-src/food/*.lps              →  只由 --convert-food 一次性迁移到 food.json
 *   vup.json.profile                   →  apps/desktop/public/pet/pet.json
 *                                          apps/desktop/public/pet/manifest.json
 *
 * 目录 → 动画信息的推断规则移植自 legacy/VPet-Simulator.Core/Handle/PetLoader.cs（LoadGraph）
 * 与 legacy/VPet-Simulator.Core/Graph/GraphInfo.cs（GraphInfo(path, info)），见 docs/04-body-animation.md §1。
 *
 * 用法：
 *   node scripts/build-assets.mjs --convert-pet [--pet vup]
 *   node scripts/build-assets.mjs --convert-food
 *   node scripts/build-assets.mjs [--pet vup] [--size 500] [--quality 85] [--force] [--dry]
 */
import fs from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const args = parseArgs(process.argv.slice(2))
const PET = args.pet ?? 'vup'
const SIZE = Number(args.size ?? 500)
const QUALITY = Number(args.quality ?? 85)
const FORCE = Boolean(args.force)
const DRY = Boolean(args.dry)
const CONVERT_PET = Boolean(args['convert-pet'])
const CONVERT_FOOD = Boolean(args['convert-food'])

const SRC_ROOT = path.join(ROOT, 'assets-src', 'pet')
const PET_DIR = path.join(SRC_ROOT, PET)
const PET_LPS = path.join(SRC_ROOT, `${PET}.lps`)
const PET_JSON = path.join(SRC_ROOT, `${PET}.json`)
const OUT_DIR = path.join(ROOT, 'apps', 'desktop', 'public', 'pet')
const FOOD_DIR = path.join(ROOT, 'assets-src', 'food')
const FOOD_JSON = path.join(FOOD_DIR, 'food.json')
const FOOD_CATEGORIES = ['gifts', 'foods', 'drinks', 'medicines']
const CORE_FOOD_CATALOG = path.join(ROOT, 'apps', 'desktop', 'src-tauri', 'food-catalog.json')
/** 食物精灵在 500 的画布里最宽也就 ~65 逻辑像素，128 够 2 倍屏用了 */
const FOOD_SIZE = 128

/** 与 packages/shared/src/manifest.ts 的 GRAPH_TYPES 同序（原版 GraphType 枚举顺序） */
const GRAPH_TYPES = [
  'common', 'raised_dynamic', 'raised_static', 'move', 'default', 'touch_head', 'touch_body',
  'idel', 'sleep', 'say', 'stateone', 'statetwo', 'startup', 'shutdown', 'work',
  'switch_up', 'switch_down', 'switch_thirsty', 'switch_hunger',
  'sidehide_left_main', 'sidehide_left_rise', 'sidehide_right_main', 'sidehide_right_rise',
]
const GRAPH_TYPE_TOKENS = GRAPH_TYPES.map((t) => t.split('_'))
const MOODS = ['happy', 'nomal', 'poorcondition', 'ill']
/** 与 packages/shared/src/pet.ts 一致；夹心动画的前后层也要按心情降级查找。 */
const MOOD_FALLBACK = {
  happy: ['happy', 'nomal', 'poorcondition', 'ill'],
  nomal: ['nomal', 'happy', 'poorcondition', 'ill'],
  poorcondition: ['poorcondition', 'nomal', 'ill', 'happy'],
  ill: ['ill', 'poorcondition', 'nomal', 'happy'],
}
const ANIMATS = { single: 'single', a_start: 'start', b_loop: 'loop', c_end: 'end' }
const GRAPH_LOADERS = new Set(['pnganimation', 'apnganimation', 'picture', 'foodanimation'])

main().catch((e) => {
  console.error(e)
  process.exit(1)
})

async function main() {
  const t0 = Date.now()
  await assertDir(PET_DIR, `找不到宠物资产目录 ${PET_DIR}`)

  // LPS 只在迁移命令中读取；正常构建只认显式的 JSON 映射。
  if (CONVERT_PET) {
    await convertPetSource()
    return
  }
  if (CONVERT_FOOD) {
    await convertFoodSource()
    return
  }

  // 1. JSON 映射 → 原始 clip 列表
  if (!(await exists(PET_JSON))) {
    throw new Error(`找不到 ${PET_JSON}\n先运行 pnpm convert:pet，把 vup.lps / info.lps 迁移为 JSON。`)
  }
  const petSource = await readPetSource()
  const raw = await hydratePetSource(petSource)
  const frameCount = raw.reduce((n, c) => n + (c.files?.length ?? 0), 0)
  console.log(`读取 ${path.relative(ROOT, PET_JSON)}：${raw.filter((c) => !c._layered).length} 段动画，${frameCount} 帧，${raw.filter((c) => c._layered).length} 段夹心`)

  // 2. 分组成变体，生成 clip id
  const layeredRaw = raw.filter((c) => c._layered)
  const groups = new Map()
  for (const c of raw.filter((c) => !c._layered).sort((a, b) => a.source.localeCompare(b.source))) {
    const key = `${c.type}/${c.name}/${c.mood}/${c.animat}`
    if (!groups.has(key)) groups.set(key, [])
    groups.get(key).push(c)
  }
  const clips = []
  const index = {}
  for (const [key, list] of groups) {
    index[key] = []
    list.forEach((c, variant) => {
      const id = `${key}/${variant}`
      index[key].push(id)
      clips.push({
        id,
        type: c.type,
        name: c.name,
        mood: c.mood,
        animat: c.animat,
        variant,
        ...(c.layer ? { layer: c.layer } : {}),
        frames: c.files.map((f, i) => ({ src: `${id}/${String(i).padStart(3, '0')}.webp`, ms: f.ms })),
        totalMs: c.files.reduce((n, f) => n + f.ms, 0),
        source: path.relative(PET_DIR, c.dir).replaceAll('\\', '/'),
        _files: c.files,
      })
    })
  }

  // 3. 转码
  if (!DRY) {
    const sharp = (await import('sharp')).default
    sharp.concurrency(1) // 我们自己做并发，避免线程数爆炸
    await fs.mkdir(OUT_DIR, { recursive: true })
    const jobs = []
    for (const clip of clips) {
      const dir = path.join(OUT_DIR, clip.id)
      clip._files.forEach((f, i) => jobs.push({ clip, i, src: f.path, dst: path.join(dir, `${String(i).padStart(3, '0')}.webp`) }))
    }
    const failed = []
    let done = 0
    let skipped = 0
    const total = jobs.length
    const workers = Math.max(2, Math.min(os.cpus().length, 12))
    console.log(`转码 ${total} 帧 → ${SIZE}×${SIZE} WebP(q${QUALITY})，${workers} 并发…`)
    await Promise.all(
      Array.from({ length: workers }, async () => {
        for (;;) {
          const job = jobs.pop()
          if (!job) return
          const pngFallback = job.dst.replace(/\.webp$/, '.png')
          const usePng = () => { job.clip.frames[job.i].src = job.clip.frames[job.i].src.replace(/\.webp$/, '.png') }
          if (!FORCE && (await isFresh(job.src, job.dst))) {
            skipped++
          } else if (!FORCE && (await isFresh(job.src, pngFallback))) {
            skipped++
            usePng()
          } else {
            await fs.mkdir(path.dirname(job.dst), { recursive: true })
            try {
              await sharp(job.src, { failOn: 'none' })
                .resize(SIZE, SIZE, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
                .webp({ quality: QUALITY, alphaQuality: 100, effort: 4 })
                .toFile(job.dst)
            } catch (e) {
              // 个别源文件 libspng 读不了：原样拷贝 PNG，manifest 里把这帧指向 .png（浏览器能解）
              failed.push(`${path.relative(PET_DIR, job.src)} — ${e.message}`)
              await fs.copyFile(job.src, pngFallback)
              usePng()
            }
          }
          done++
          if (done % 500 === 0) console.log(`  ${done}/${total}`)
        }
      }),
    )
    console.log(`转码完成：${done - skipped} 新生成，${skipped} 跳过（未变化）`)
    if (failed.length) {
      console.warn(`\n[warn] ${failed.length} 帧无法用 sharp 转码，已原样拷贝为 PNG：`)
      for (const f of failed) console.warn('  ' + f)
    }
  }

  // 3.5 夹心动画：把 info.lps 里写的层名解析成 clip id
  const layered = []
  for (const l of layeredRaw.sort((a, b) => a.source.localeCompare(b.source))) {
    // 原版 Drink 的 Happy / PoorCondition 复用 Nomal 的手部前层；LPS 只声明层名，
    // 没有为每种心情重复登记。和 Body 普通动画一样按 MOOD_FALLBACK 找最近可用层。
    const pick = (name) => {
      for (const mood of MOOD_FALLBACK[l.mood]) {
        const id = index[`${l.type}/${name}/${mood}/${l.animat}`]?.[0]
        if (id) return id
      }
      return undefined
    }
    const back = pick(l.backName)
    const front = pick(l.frontName)
    if (!back || !front) {
      console.warn(`  [warn] 夹心动画 ${l.name}/${l.mood} 找不到层：back=${l.backName} front=${l.frontName}`)
      continue
    }
    layered.push({
      id: `${l.type}/${l.name}/${l.mood}/${l.animat}`,
      type: l.type, name: l.name, mood: l.mood, animat: l.animat,
      back, front, food: l.food,
      source: l.source.replaceAll('\\', '/'),
    })
  }

  // 3.6 食物：夹心动画中间那层的图
  const food = await buildFood()

  // 4. pet.json（源数据已经由 vup.lps 迁移进 vup.json.profile）
  const petJson = petSource.profile

  // 5. manifest.json
  const manifest = {
    pet: PET,
    size: SIZE,
    generatedAt: new Date().toISOString(),
    clips: clips.map(({ _files, ...c }) => c),
    index,
    layered,
    food,
  }
  if (!DRY) {
    await fs.writeFile(path.join(OUT_DIR, 'manifest.json'), JSON.stringify(manifest))
    if (petJson) await fs.writeFile(path.join(OUT_DIR, 'pet.json'), JSON.stringify(petJson, null, 2))
    // Core 在 WebView 加载前就要能送礼 / 喂药，因此同一份 JSON 同步嵌进 Rust。
    await fs.writeFile(CORE_FOOD_CATALOG, `${JSON.stringify(food, null, 2)}\n`, 'utf8')
  }

  // 6. 统计
  const byType = {}
  for (const c of clips) byType[c.type] = (byType[c.type] ?? 0) + 1
  console.log('\n按类型统计：')
  for (const [t, n] of Object.entries(byType).sort((a, b) => b[1] - a[1])) console.log(`  ${t.padEnd(22)} ${n}`)
  console.log(`\nmanifest：${clips.length} clips · ${Object.keys(index).length} 键 · ${layered.length} 段夹心 · 用时 ${((Date.now() - t0) / 1000).toFixed(1)}s`)
  if (!DRY) console.log(`输出：${OUT_DIR}`)
}

/* ------------------------------------------------------------------ *
 * 宠物源配置：LPS 只负责一次迁移，日常构建只读取 JSON
 * ------------------------------------------------------------------ */
async function convertPetSource() {
  if (!(await exists(PET_LPS))) throw new Error(`找不到迁移源 ${PET_LPS}`)

  const raw = []
  await loadGraphDir(PET_DIR, PET_DIR, raw)
  const profile = lpsToJson(parseLps(await fs.readFile(PET_LPS, 'utf8')))
  const source = {
    $schema: '../../schemas/pet-source-v1.schema.json',
    schemaVersion: 1,
    pet: PET,
    generatedFrom: [`${PET}.lps`, `${PET}/**/info.lps`],
    profile,
    animations: raw
      .filter((entry) => !entry._layered)
      .map((entry) => ({
        source: sourcePath(entry.dir),
        type: entry.type,
        name: entry.name,
        mood: entry.mood,
        segment: entry.animat,
        ...(entry.layer ? { layer: entry.layer } : {}),
        frames: entry.files.map((frame) => ({
          file: path.basename(frame.path),
          ms: frame.ms,
        })),
      }))
      .sort(compareSourceEntry),
    layered: raw
      .filter((entry) => entry._layered)
      .map((entry) => ({
        source: entry.source.replaceAll('\\', '/'),
        type: entry.type,
        name: entry.name,
        mood: entry.mood,
        segment: entry.animat,
        layers: { back: entry.backName, front: entry.frontName },
        food: entry.food,
      }))
      .sort(compareSourceEntry),
  }

  validatePetSource(source)
  await fs.writeFile(PET_JSON, `${JSON.stringify(source, null, 2)}\n`, 'utf8')
  const frames = source.animations.reduce((count, entry) => count + entry.frames.length, 0)
  console.log(`已生成 ${path.relative(ROOT, PET_JSON)}`)
  console.log(`  ${source.animations.length} 段动画 · ${frames} 帧 · ${source.layered.length} 段夹心动画`)
  console.log('  正常构建从现在起只读取这份 JSON；原 LPS 保留作迁移依据。')
}

async function readPetSource() {
  let source
  try {
    source = JSON.parse(await fs.readFile(PET_JSON, 'utf8'))
  } catch (error) {
    throw new Error(`${PET_JSON} 不是有效 JSON：${error.message}`)
  }
  validatePetSource(source)
  return source
}

function validatePetSource(source) {
  if (!source || source.schemaVersion !== 1 || source.pet !== PET) {
    throw new Error(`${PET_JSON} 的 schemaVersion / pet 不匹配`)
  }
  if (!source.profile || !Array.isArray(source.animations) || !Array.isArray(source.layered)) {
    throw new Error(`${PET_JSON} 缺少 profile / animations / layered`)
  }

  const validType = new Set(GRAPH_TYPES)
  const validMood = new Set(MOODS)
  const validSegment = new Set(Object.values(ANIMATS))
  for (const entry of [...source.animations, ...source.layered]) {
    if (!isSafeRelativePath(entry.source)) throw new Error(`非法动画 source：${entry.source}`)
    if (!validType.has(entry.type)) throw new Error(`未知动画 type：${entry.type}`)
    if (!validMood.has(entry.mood)) throw new Error(`未知动画 mood：${entry.mood}`)
    if (!validSegment.has(entry.segment)) throw new Error(`未知动画 segment：${entry.segment}`)
    if (typeof entry.name !== 'string' || !entry.name) throw new Error(`动画缺少 name：${entry.source}`)
  }
  for (const entry of source.animations) {
    if (!Array.isArray(entry.frames) || entry.frames.length === 0) throw new Error(`动画没有 frames：${entry.source}`)
    for (const frame of entry.frames) {
      if (!isSafeFileName(frame.file) || !Number.isFinite(frame.ms) || frame.ms <= 0) {
        throw new Error(`非法动画帧：${entry.source}/${frame.file}`)
      }
    }
  }
  for (const entry of source.layered) {
    if (!entry.layers?.back || !entry.layers?.front || !Array.isArray(entry.food)) {
      throw new Error(`夹心动画缺少 layers / food：${entry.source}`)
    }
  }
}

async function hydratePetSource(source) {
  const out = []
  for (const entry of source.animations) {
    const dir = path.join(PET_DIR, ...entry.source.split('/'))
    const files = entry.frames.map((frame) => ({
      path: path.join(dir, frame.file),
      ms: frame.ms,
    }))
    const missing = []
    for (const frame of files) if (!(await exists(frame.path))) missing.push(path.basename(frame.path))
    if (missing.length) throw new Error(`动画 ${entry.source} 缺少帧：${missing.slice(0, 5).join(', ')}`)
    out.push({
      type: entry.type,
      name: entry.name,
      mood: entry.mood,
      animat: entry.segment,
      ...(entry.layer ? { layer: entry.layer } : {}),
      dir,
      source: entry.source,
      files,
    })
  }
  for (const entry of source.layered) {
    out.push({
      type: entry.type,
      name: entry.name,
      mood: entry.mood,
      animat: entry.segment,
      _layered: true,
      dir: path.join(PET_DIR, ...entry.source.split('/')),
      backName: entry.layers.back,
      frontName: entry.layers.front,
      food: entry.food,
      source: entry.source,
    })
  }
  return out
}

function sourcePath(dir) {
  return path.relative(PET_DIR, dir).replaceAll('\\', '/')
}

function compareSourceEntry(a, b) {
  return `${a.source}\0${a.type}\0${a.name}\0${a.mood}\0${a.segment}`
    .localeCompare(`${b.source}\0${b.type}\0${b.name}\0${b.mood}\0${b.segment}`)
}

function isSafeRelativePath(value) {
  return typeof value === 'string' && value !== '' && !path.isAbsolute(value) && !value.split(/[\\/]/).includes('..')
}

function isSafeFileName(value) {
  return typeof value === 'string' && value !== '' && path.basename(value) === value && /\.png$/i.test(value)
}

/* ------------------------------------------------------------------ *
 * PetLoader.LoadGraph 的移植：递归目录，info.lps 优先，叶子目录自动生成
 * ------------------------------------------------------------------ */
async function loadGraphDir(dir, startup, out) {
  const entries = await fs.readdir(dir, { withFileTypes: true })
  const subdirs = entries.filter((e) => e.isDirectory())
  const infoPath = path.join(dir, 'info.lps')

  if (await exists(infoPath)) {
    const lines = parseLps(await fs.readFile(infoPath, 'utf8'))
    for (const line of lines) {
      if (!GRAPH_LOADERS.has(line.name.toLowerCase())) continue
      // 夹心动画：本身没有帧，只声明「后层 + 中间食物轨迹 + 前层」
      if (line.name.toLowerCase() === 'foodanimation') {
        out.push({
          ...graphInfo(dir, true, line, startup),
          _layered: true,
          dir,
          backName: (line.subs.back_lay ?? '').toLowerCase(),
          frontName: (line.subs.front_lay ?? '').toLowerCase(),
          food: parseFoodKeyframes(line.subs),
          source: path.relative(startup, dir),
        })
        continue
      }
      const rel = line.subs.path
      const p = rel ? path.join(dir, rel.replaceAll('\\', path.sep)) : dir
      const st = await fs.stat(p).catch(() => null)
      if (!st) {
        console.warn(`  [warn] info.lps 指向不存在的路径：${p}`)
        continue
      }
      if (st.isDirectory()) await addClipFromDir(p, startup, line, out)
      else await addClipFromFiles(path.dirname(p), [p], startup, line, out, /* isFile */ true)
    }
    return // 有 info.lps 的目录不再自动向下扫描（与原版一致）
  }

  if (subdirs.length === 0) {
    await addClipFromDir(dir, startup, emptyLine(), out)
    return
  }
  for (const d of subdirs) await loadGraphDir(path.join(dir, d.name), startup, out)
}

async function addClipFromDir(dir, startup, line, out) {
  const files = (await fs.readdir(dir))
    .filter((f) => /\.png$/i.test(f))
    .sort()
    .map((f) => path.join(dir, f))
  if (files.length === 0) return
  await addClipFromFiles(dir, files, startup, line, out, false)
}

async function addClipFromFiles(dir, files, startup, line, out, isFile) {
  const info = graphInfo(isFile ? files[0] : dir, !isFile, line, startup)
  out.push({
    ...info,
    dir,
    source: path.relative(startup, dir),
    files: files.map((p) => ({ path: p, ms: frameMs(p) })),
  })
}

/** JSON 食物目录 → 夹心动画中间层；正常构建不再读取 `.lps`。 */
async function buildFood() {
  const source = await readFoodSource()
  const items = FOOD_CATEGORIES.flatMap((category) => source.categories[category])
    .sort((a, b) => a.id.localeCompare(b.id))
  const out = []
  const outDir = path.join(OUT_DIR, 'food')
  if (!DRY) await fs.mkdir(outDir, { recursive: true })
  const sharp = DRY ? null : (await import('sharp')).default
  let missing = 0
  for (const it of items) {
    const original = path.join(FOOD_DIR, it.image)
    if (!(await exists(original))) { missing++; continue }
    const id = it.id
    const src = `food/${id}.webp`
    if (!DRY) {
      const dst = path.join(OUT_DIR, src)
      if (FORCE || !(await exists(dst))) {
        await sharp(original)
          .resize(FOOD_SIZE, FOOD_SIZE, { fit: 'inside', withoutEnlargement: true })
          .webp({ quality: QUALITY })
          .toFile(dst)
      }
    }
    const { image, source: _source, ...rest } = it
    out.push({ id, src, ...rest })
  }
  console.log(`食物：${out.length} 项${missing ? `（${missing} 项缺图，已跳过）` : ''}`)
  return out
}

/**
 * 原版五份 LPS 只迁移一次。分类层负责 UI 与后续维护：Gift / Drug 优先，
 * 其余再按夹心动画分成饮料和食物；Functional 因此不会被误塞进单独的“其他”。
 */
async function convertFoodSource() {
  const lpsFiles = (await fs.readdir(FOOD_DIR).catch(() => []))
    .filter((file) => file.toLowerCase().endsWith('.lps')).sort()
  if (!lpsFiles.length) throw new Error(`找不到迁移源 ${FOOD_DIR}/*.lps`)
  const categories = Object.fromEntries(FOOD_CATEGORIES.map((name) => [name, []]))
  const names = new Set()
  let index = 0
  for (const file of lpsFiles) {
    for (const line of parseLps(await fs.readFile(path.join(FOOD_DIR, file), 'utf8'))) {
      if (line.name !== 'food') continue
      const name = line.subs.name
      const graph = (line.subs.graph ?? '').toLowerCase()
      const type = line.subs.type ?? ''
      if (!name || !graph) continue
      if (names.has(name)) throw new Error(`食物名重复：${name}`)
      names.add(name)
      const category = type === 'Gift' || graph === 'gift'
        ? 'gifts'
        : type === 'Drug'
          ? 'medicines'
          : graph === 'drink' ? 'drinks' : 'foods'
      categories[category].push({
        id: String(index++).padStart(3, '0'),
        name,
        category,
        graph,
        type,
        image: `image/${name}.png`,
        strength: num(line.subs.Strength),
        strengthFood: num(line.subs.StrengthFood),
        strengthDrink: num(line.subs.StrengthDrink),
        feeling: num(line.subs.Feeling),
        health: num(line.subs.Health),
        price: num(line.subs.price),
        exp: num(line.subs.Exp),
        likability: num(line.subs.Likability),
        description: line.subs.desc ?? '',
        source: file,
      })
    }
  }
  const source = {
    $schema: '../../schemas/food-source-v1.schema.json',
    schemaVersion: 1,
    generatedFrom: lpsFiles,
    categories,
  }
  validateFoodSource(source)
  await fs.writeFile(FOOD_JSON, `${JSON.stringify(source, null, 2)}\n`, 'utf8')
  console.log(`已生成 ${path.relative(ROOT, FOOD_JSON)}`)
  console.log(FOOD_CATEGORIES.map((category) => `${category} ${categories[category].length}`).join(' · '))
  console.log('正常构建从现在起只读取 food.json；原 LPS 保留作迁移依据。')
}

async function readFoodSource() {
  if (!(await exists(FOOD_JSON))) {
    throw new Error(`找不到 ${FOOD_JSON}\n先运行 pnpm convert:food，把 food/*.lps 迁移为 JSON。`)
  }
  let source
  try {
    source = JSON.parse(await fs.readFile(FOOD_JSON, 'utf8'))
  } catch (error) {
    throw new Error(`${FOOD_JSON} 不是有效 JSON：${error.message}`)
  }
  validateFoodSource(source)
  return source
}

function validateFoodSource(source) {
  if (!source || source.schemaVersion !== 1 || !source.categories) {
    throw new Error(`${FOOD_JSON} 的 schemaVersion / categories 不匹配`)
  }
  const ids = new Set()
  const names = new Set()
  for (const category of FOOD_CATEGORIES) {
    if (!Array.isArray(source.categories[category])) throw new Error(`${FOOD_JSON} 缺少分类 ${category}`)
    for (const item of source.categories[category]) {
      if (!item?.id || !item.name || !item.graph || !item.image || item.category !== category) {
        throw new Error(`${FOOD_JSON} 的 ${category} 中有无效项目`)
      }
      if (ids.has(item.id) || names.has(item.name)) {
        throw new Error(`${FOOD_JSON} 有重复 id / name：${item.id} ${item.name}`)
      }
      ids.add(item.id)
      names.add(item.name)
    }
  }
}

const num = (v) => (v === undefined ? 0 : Number.parseFloat(v) || 0)

/**
 * 食物精灵的运动轨迹：`aN#时长,x,y,宽,旋转,不透明度`
 * （移植自 legacy FoodAnimation.Animation(ISub) 的构造）。
 * 只给一个值表示这段时间食物不显示，如 `a8#750`。
 */
function parseFoodKeyframes(subs) {
  const out = []
  for (let i = 0; subs[`a${i}`] !== undefined; i++) {
    const n = subs[`a${i}`].split(',').map((s) => Number.parseFloat(s))
    if (n.length === 1) out.push({ ms: n[0], visible: false })
    else {
      out.push({
        ms: n[0], visible: true, x: n[1], y: n[2], width: n[3],
        rotate: n.length > 4 ? n[4] : 0,
        opacity: n.length > 5 ? n[5] : 1,
      })
    }
  }
  return out
}

/** 帧时长：文件名最后一个 `_` 之后的数字（PNGAnimation.cs:227）；单图默认 1000ms */
function frameMs(file) {
  const base = path.basename(file, path.extname(file))
  const i = base.lastIndexOf('_')
  const n = i >= 0 ? Number.parseInt(base.slice(i + 1), 10) : NaN
  return Number.isFinite(n) && n > 0 ? n : 1000
}

/* ------------------------------------------------------------------ *
 * GraphInfo(path, info) 的移植
 * ------------------------------------------------------------------ */
function graphInfo(fsPath, isDir, line, startup) {
  const full = (isDir ? fsPath : fsPath.slice(0, fsPath.length - path.extname(fsPath).length)).toLowerCase()
  const rel = full.split(startup.toLowerCase()).pop() ?? ''
  const tokens = rel.replace(/[\\/]/g, '_').split('_').filter((t) => t.trim() !== '')

  // 1. 心情
  let mood = MOODS.find((m) => m === (line.subs.mode ?? '').toLowerCase())
  if (!mood) {
    mood = 'nomal'
    for (const m of MOODS) if (removeFirst(tokens, m)) { mood = m; break }
  }

  // 2. 类型：按枚举顺序找第一个 token 序列匹配
  let type = GRAPH_TYPES.find((t) => t === (line.subs.graph ?? '').toLowerCase())
  if (!type) {
    type = 'common'
    for (let i = 0; i < GRAPH_TYPE_TOKENS.length; i++) {
      const seq = GRAPH_TYPE_TOKENS[i]
      const at = tokens.indexOf(seq[0])
      if (at < 0) continue
      let ok = true
      for (let b = 1; b < seq.length && at + b < tokens.length; b++) if (tokens[at + b] !== seq[b]) { ok = false; break }
      if (ok) { type = GRAPH_TYPES[i]; tokens.splice(at, seq.length); break }
    }
  }

  // 3. 段落
  let animat = ANIMATS[(line.subs.animat ?? '').toLowerCase()]
  if (!animat) {
    if (removeFirst(tokens, 'a') || removeFirst(tokens, 'start')) animat = 'start'
    else if (removeFirst(tokens, 'b') || removeFirst(tokens, 'loop')) animat = 'loop'
    else if (removeFirst(tokens, 'c') || removeFirst(tokens, 'end')) animat = 'end'
    else { removeFirst(tokens, 'single'); animat = 'single' }
  }

  // 4. 名字
  let name = (line.info ?? '').trim().toLowerCase()
  if (!name) {
    while (tokens.length && (isNumeric(tokens.at(-1)) || tokens.at(-1).startsWith('~'))) tokens.pop()
    name = tokens.at(-1) ?? ''
  }
  if (!name) name = type

  // 带变体后缀的也算，如 eat_back_lay_2（info.lps 里就是这么命名的）
  const layer = /back_lay(_\d+)?$/.test(name) ? 'back' : /front_lay(_\d+)?$/.test(name) ? 'front' : undefined
  return { type, name, mood, animat, layer }
}

/* ------------------------------------------------------------------ *
 * LPS（LinePutScript）最小解析：每行 `name#info:|k#v:|k#v:|`，`///` 开头是注释
 * ------------------------------------------------------------------ */
function parseLps(text) {
  const lines = []
  for (const rawLine of text.split(/\r?\n/)) {
    const l = rawLine.trim()
    if (!l || l.startsWith('///')) continue
    const parts = l.split(':|').filter((p) => p !== '')
    if (parts.length === 0) continue
    const [name, info] = splitKV(parts[0])
    const subs = {}
    for (const p of parts.slice(1)) {
      const [k, v] = splitKV(p)
      subs[k] = v
    }
    lines.push({ name, info, subs })
  }
  return lines
}
function splitKV(s) {
  const i = s.indexOf('#')
  return i < 0 ? [s, ''] : [s.slice(0, i), s.slice(i + 1)]
}
function emptyLine() {
  return { name: 'pnganimation', info: '', subs: {} }
}
/** vup.lps → JSON：同名行合并为数组（work / move），其他为对象；数字自动转 number */
function lpsToJson(lines) {
  const out = {}
  for (const line of lines) {
    const obj = {}
    if (line.info) obj._ = coerce(line.info)
    for (const [k, v] of Object.entries(line.subs)) obj[k] = coerce(v)
    if (line.name in out) {
      if (!Array.isArray(out[line.name])) out[line.name] = [out[line.name]]
      out[line.name].push(obj)
    } else out[line.name] = obj
  }
  for (const k of ['work', 'move']) if (out[k] && !Array.isArray(out[k])) out[k] = [out[k]]
  return out
}
function coerce(v) {
  return /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : v
}

/* ------------------------------------------------------------------ *
 * 小工具
 * ------------------------------------------------------------------ */
function removeFirst(arr, v) {
  const i = arr.indexOf(v)
  if (i < 0) return false
  arr.splice(i, 1)
  return true
}
function isNumeric(s) {
  return /^-?\d+(\.\d+)?$/.test(s)
}
async function exists(p) {
  return fs.access(p).then(() => true, () => false)
}
async function assertDir(p, msg) {
  const st = await fs.stat(p).catch(() => null)
  if (!st?.isDirectory()) throw new Error(msg)
}
async function isFresh(src, dst) {
  const [a, b] = await Promise.all([fs.stat(src).catch(() => null), fs.stat(dst).catch(() => null)])
  return Boolean(a && b && b.mtimeMs >= a.mtimeMs)
}
function parseArgs(argv) {
  const o = {}
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (!a.startsWith('--')) continue
    const k = a.slice(2)
    const next = argv[i + 1]
    if (next && !next.startsWith('--')) { o[k] = next; i++ } else o[k] = true
  }
  return o
}
