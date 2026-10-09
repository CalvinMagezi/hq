/** An inline error in the app's red token. */
export function ErrorText({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return (
    <p role="alert" className={`text-[11px] font-mono break-words ${className}`} style={{ color: 'var(--accent-red)' }}>
      {children}
    </p>
  )
}
