/** An inline error in amber: a failure is something to look at, not a destroyed thing. */
export function ErrorText({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return (
    <p role="alert" className={`a-body-sm a-warn-t break-words ${className}`}>
      {children}
    </p>
  )
}
