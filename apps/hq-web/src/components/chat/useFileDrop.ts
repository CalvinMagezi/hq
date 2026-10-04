import { useRef, useState } from 'react'

/** Drag-and-drop of files onto an area. `dragging` drives the drop hint. */
export function useFileDrop(onFiles: (files: File[]) => void) {
  const [dragging, setDragging] = useState(false)
  const depth = useRef(0)
  const carriesFiles = (e: React.DragEvent) => e.dataTransfer.types.includes('Files')

  const handlers = {
    onDragEnter: (e: React.DragEvent) => {
      if (!carriesFiles(e)) return
      depth.current += 1
      setDragging(true)
    },
    onDragOver: (e: React.DragEvent) => {
      if (carriesFiles(e)) e.preventDefault()
    },
    // Enter and leave fire for every child crossed, so only the last leave ends the drag.
    onDragLeave: () => {
      depth.current = Math.max(0, depth.current - 1)
      if (depth.current === 0) setDragging(false)
    },
    onDrop: (e: React.DragEvent) => {
      e.preventDefault()
      depth.current = 0
      setDragging(false)
      const files = Array.from(e.dataTransfer.files)
      if (files.length > 0) onFiles(files)
    },
  }
  return { dragging, handlers }
}
