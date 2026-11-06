import { useCallback, useEffect, useRef, useState } from 'react'
import { Loader2, Send } from 'lucide-react'
import type { TaskComment } from '~/lib/tasksApi'
import { addCommentClient, fetchCommentsClient, parseSqliteUtc } from '~/lib/tasksApi'
import { useWS } from '~/context/WebSocketContext'
import { t } from '~/strings'
import { SectionLabel } from './taskFields'
import { MarkdownViewer } from '../MarkdownViewer'
import './leaf.css'

/** A task's comment thread: loads, follows other tabs' comments live, and posts new ones. */
export function TaskComments({ taskId }: { taskId: string }) {
  const [comments, setComments] = useState<TaskComment[]>([])
  const [commentText, setCommentText] = useState('')
  const [postingComment, setPostingComment] = useState(false)
  const { subscribe } = useWS()

  // One post can set off several reloads; only the newest request may set the list.
  const latestLoad = useRef(0)

  const loadComments = useCallback(async () => {
    const load = ++latestLoad.current
    try {
      const res = await fetchCommentsClient(taskId)
      if (load === latestLoad.current) setComments(res.comments)
    } catch (e) {
      console.error('Failed to load comments:', e)
    }
  }, [taskId])

  useEffect(() => {
    loadComments()
    return subscribe((msg) => {
      // task:sync covers comments an agent added over MCP, which have no REST request to announce them.
      if ((msg.type === 'task:comment_added' && msg.task_id === taskId) || msg.type === 'task:sync') loadComments()
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
      <div className="a-card l-lines mb-3">
        {comments.map((c) => (
          <div key={c.id} className="a-drow l-tall l-col">
            <div className="l-row justify-between">
              <span className="a-body-sm-b">{c.author}</span>
              <span className="a-cap a-muted">
                {parseSqliteUtc(c.created_at).toLocaleString()}
              </span>
            </div>
            <div className="a-body-sm">
              <MarkdownViewer content={c.body} bare />
            </div>
          </div>
        ))}
        {comments.length === 0 && <p className="a-body-sm a-muted l-pad">No comments yet</p>}
      </div>
      <div className="flex items-center gap-2">
        <input
          value={commentText}
          onChange={(e) => setCommentText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') handlePostComment()
          }}
          placeholder="Add a comment..."
          className="a-input flex-1"
        />
        <button
          type="button"
          onClick={handlePostComment}
          disabled={postingComment || !commentText.trim()}
          aria-label={t('chat_send')}
          className="a-ib sm"
        >
          {postingComment ? <Loader2 className="a-ic sm animate-spin" aria-hidden="true" /> : <Send className="a-ic sm" aria-hidden="true" />}
        </button>
      </div>
    </div>
  )
}
