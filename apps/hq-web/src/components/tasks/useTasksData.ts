import { useCallback, useEffect, useState } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useHQStore } from '~/store/hqStore'
import { fetchFoldersClient, fetchInitiativesClient, fetchSpacesClient, fetchTasksClient } from '~/lib/tasksApi'
import { useWS } from '~/context/WebSocketContext'

/** The tasks page's store slice, plus loaders that refresh it from the server. */
export function useTasksData() {
  const store = useHQStore(
    useShallow((s) => ({
      tasks: s.tasks,
      setTasks: s.setTasks,
      upsertTask: s.upsertTask,
      removeTask: s.removeTask,
      spaces: s.spaces,
      setSpaces: s.setSpaces,
      folders: s.folders,
      setFolders: s.setFolders,
      initiatives: s.initiatives,
      setInitiatives: s.setInitiatives,
      selectedTaskId: s.selectedTaskId,
      setSelectedTaskId: s.setSelectedTaskId,
    })),
  )
  const { setTasks, setSpaces, setFolders, setInitiatives } = store
  const [isRefreshing, setIsRefreshing] = useState(false)
  const { subscribe } = useWS()

  const loadTaxonomy = useCallback(async () => {
    try {
      const [spacesRes, foldersRes, initiativesRes] = await Promise.all([
        fetchSpacesClient(),
        fetchFoldersClient(),
        fetchInitiativesClient(),
      ])
      setSpaces(spacesRes.spaces)
      setFolders(foldersRes.folders)
      setInitiatives(initiativesRes.initiatives)
    } catch (e) {
      console.error('Failed to load spaces/folders/initiatives:', e)
    }
  }, [setSpaces, setFolders, setInitiatives])

  const loadTasks = useCallback(async () => {
    setIsRefreshing(true)
    try {
      const res = await fetchTasksClient({})
      setTasks(res.tasks)
    } catch (e) {
      console.error('Failed to load tasks:', e)
    } finally {
      setIsRefreshing(false)
    }
  }, [setTasks])

  useEffect(() => {
    loadTaxonomy()
    loadTasks()
  }, [loadTaxonomy, loadTasks])

  // Fallback for task mutations that didn't originate in this browser tab
  // (agent tool calls via MCP): tasks_watch.rs polls the DB and broadcasts
  // task:sync; the event carries no per-task delta, so a change refetches all.
  useEffect(() => {
    return subscribe((msg) => {
      if (msg.type === 'task:sync') loadTasks()
    })
  }, [subscribe, loadTasks])

  return { ...store, isRefreshing, loadTaxonomy, loadTasks }
}
