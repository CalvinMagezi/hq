import { Component, type ReactNode } from 'react'
import { RotateCw } from 'lucide-react'

interface Props {
  children: ReactNode
  /** The message's own text, shown as plain text so a rendering failure never hides what was written. */
  plainText?: string
  /** A new value clears the error (say, a reply that grew after the bad part streamed in). */
  resetKey?: unknown
}

interface State {
  failed: boolean
}

/** Keeps one message that cannot render from taking the whole chat down with it. */
export class StreamErrorBoundary extends Component<Props, State> {
  state: State = { failed: false }

  static getDerivedStateFromError(): State {
    return { failed: true }
  }

  componentDidCatch(error: unknown) {
    console.error('StreamErrorBoundary: a chat message failed to render', error)
  }

  componentDidUpdate(prev: Props) {
    if (this.state.failed && prev.resetKey !== this.props.resetKey) this.setState({ failed: false })
  }

  render() {
    if (!this.state.failed) return this.props.children
    return (
      <div role="alert" className="rounded-2xl border border-amber-500/30 bg-amber-500/[0.04] px-3.5 py-3 text-sm font-mono">
        <p className="text-neutral-200">This message could not be displayed. Nothing was lost.</p>
        {this.props.plainText && (
          <pre className="mt-2 max-h-64 overflow-y-auto whitespace-pre-wrap break-words text-[13px] text-neutral-300">
            {this.props.plainText}
          </pre>
        )}
        <button
          type="button"
          onClick={() => this.setState({ failed: false })}
          className="mt-2 flex items-center gap-1.5 h-9 px-3 rounded-full border border-white/15 text-xs text-neutral-200 hover:bg-white/10"
        >
          <RotateCw className="w-3.5 h-3.5" /> Try again
        </button>
      </div>
    )
  }
}
