import { Fragment, type ReactNode } from 'react'

function pendingCode(state: { lines: string[] | null }): string[] | null {
  return state.lines
}

function inline(text: string): ReactNode[] {
  const parts = text.split(/(`[^`]*`|\*\*[^*]+\*\*|__[^_]+__|\*[^*]+\*|_[^_]+_|~~[^~]+~~|\[[^\]]+\]\(https?:\/\/[^)\s]+\))/g)
  return parts.map((part, i) => {
    if (part.startsWith('`') && part.endsWith('`')) return <code key={i}>{part.slice(1, -1)}</code>
    if ((part.startsWith('**') && part.endsWith('**')) || (part.startsWith('__') && part.endsWith('__'))) {
      return <strong key={i}>{part.slice(2, -2)}</strong>
    }
    if ((part.startsWith('*') && part.endsWith('*')) || (part.startsWith('_') && part.endsWith('_'))) {
      return <em key={i}>{part.slice(1, -1)}</em>
    }
    if (part.startsWith('~~') && part.endsWith('~~')) return <del key={i}>{part.slice(2, -2)}</del>
    const link = part.match(/^\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)$/)
    if (link) return <a key={i} href={link[2]} target="_blank" rel="noreferrer">{link[1]}</a>
    return <Fragment key={i}>{part}</Fragment>
  })
}

export function MarkdownContent({ text }: { text: string }) {
  const lines = text.replace(/\r\n?/g, '\n').split('\n')
  const blocks: ReactNode[] = []
  let paragraph: string[] = []
  let list: string[] = []
  let quote: string[] = []
  const codeState: { lines: string[] | null } = { lines: null }

  const flushParagraph = () => {
    if (paragraph.length) {
      blocks.push(<p key={`p-${blocks.length}`}>{paragraph.map((line, i) => <Fragment key={i}>{i > 0 && <br />}{inline(line)}</Fragment>)}</p>)
      paragraph = []
    }
  }
  const flushList = () => {
    if (list.length) {
      blocks.push(<ul key={`ul-${blocks.length}`}>{list.map((item, i) => <li key={i}>{inline(item)}</li>)}</ul>)
      list = []
    }
  }
  const flushQuote = () => {
    if (quote.length) {
      blocks.push(<blockquote key={`quote-${blocks.length}`}>{quote.map((line, i) => <Fragment key={i}>{i > 0 && <br />}{inline(line)}</Fragment>)}</blockquote>)
      quote = []
    }
  }

  lines.forEach((line, index) => {
    if (codeState.lines) {
      if (/^\s*```/.test(line)) {
        blocks.push(<pre key={`code-${index}`}><code>{codeState.lines.join('\n')}</code></pre>)
        codeState.lines = null
      } else codeState.lines.push(line)
      return
    }
    if (/^\s*```/.test(line)) {
      flushParagraph(); flushList(); flushQuote()
      codeState.lines = []
      return
    }
    const heading = line.match(/^\s{0,3}(#{1,3})\s+(.+)$/)
    if (heading) {
      flushParagraph(); flushList(); flushQuote()
      const Tag = `h${heading[1].length}` as 'h1' | 'h2' | 'h3'
      blocks.push(<Tag key={`h-${index}`}>{inline(heading[2])}</Tag>)
      return
    }
    const item = line.match(/^\s*[-*+]\s+(.+)$/)
    if (item) {
      flushParagraph(); flushQuote(); list.push(item[1])
      return
    }
    const quoted = line.match(/^\s*>\s?(.*)$/)
    if (quoted) {
      flushParagraph(); flushList(); quote.push(quoted[1])
      return
    }
    if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
      flushParagraph(); flushList(); flushQuote()
      blocks.push(<hr key={`hr-${index}`} />)
      return
    }
    if (!line.trim()) {
      flushParagraph(); flushList(); flushQuote()
      return
    }
    flushList(); flushQuote(); paragraph.push(line)
  })
  const trailingCode = pendingCode(codeState)
  if (trailingCode) blocks.push(<pre key="code-end"><code>{trailingCode.join('\n')}</code></pre>)
  flushParagraph(); flushList(); flushQuote()
  return <div className="markdown-content">{blocks}</div>
}
