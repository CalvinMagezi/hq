import { useCallback, useEffect, useState } from 'react'
import { Loader2, Send } from 'lucide-react'
import type { TaskComment } from '~/lib/tasksApi'
import { addCommentClient, fetchCommentsClient, parseSqliteUtc } from '~/lib/tasksApi'
import { useWS } from '~/context/WebSocketContext'
import { SectionLabel } from './taskFields'
import { MarkdownViewer } from '../MarkdownViewer'

/** A task's comment thread: loads, follows other tabs' comments live, and posts new ones. */
export function TaskComments({ taskId }: { taskId: string }) {
  const [comments, setComments] = useState<TaskComment[]>([])
  const [commentText, setCommentText] = useState('')
  const [postingComment, setPostingComment] = useState(false)
  const { subscribe } = useWS()

  const loadComments = useCallback(async () => {
    try {
      const res = await fetchCommentsClient(taskId)
      setComments(res.comments)
    } catch (e) {
      console.error('Failed to load comments:', e)
    }
  }, [taskId])

  useEffect(() => {
    loadComments()
    return subscribe((msg) => {
      if (msg.type === 'task:comment_added' && msg.task_id === taskId) loadComments()
    })
  }, [taskId, subscribe, loadComments])

  const handlePostComment = async () => {
    const body = commentText.trim()
    if (!body) return
    setPostingComment(true)
    try {
      await addCommentClient(taskId, body)
      setCommentText('')
      await loadComments()
    } catch (e) {
      console.error('Failed to add comment:', e)
    } finally {
      setPostingComment(false)
    }
  }

  return (
    <div>
      <SectionLabel className="mb-2">Comments ({comments.length})</SectionLabel>
      <div className="space-y-2.5 mb-3">
        {comments.map((c) => (
          <div key={c.id} className="p-3 rounded-xl bg-white/[0.02] border border-white/5">
            <div className="flex items-center justify-between mb-1">
              <span className="text-[11px] font-mono font-bold text-neutral-300">{c.author}</span>
              <span className="text-[10px] font-mono text-neutral-500">
                {parseSqliteUtc(c.created_at).toLocaleString()}
              </span>
            </div>
            <div className="text-xs text-neutral-300 leading-relaxed">
              <MarkdownViewer content={c.body} bare />
            </div>
          </div>
        ))}
        {comments.length === 0 && <p className="text-xs font-mono text-neutral-600 italic">No comments yet</p>}
      </div>
      <div className="flex items-center gap-2">
        <input
          value={commentText}
          onChange={(e) => setCommentText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') handlePostComment()
          }}
          placeholder="Add a comment..."
          className="flex-1 px-3.5 py-2 rounded-xl text-xs font-mono text-neutral-200 bg-black/30 border focus:outline-none focus:ring-1 focus:ring-emerald-400"
          style={{ borderColor: 'rgba(255,255,255,0.1)' }}
        />
        <button
          type="button"
          onClick={handlePostComment}
          disabled={postingComment || !commentText.trim()}
          className="p-2 rounded-xl text-emerald-400 hover:bg-white/10 transition-colors disabled:opacity-40"
        >
          {postingComment ? <Loader2 className="w-4 h-4 animate-spin" /> : <Send className="w-4 h-4" />}
        </button>
      </div>
    </div>
  )
}
