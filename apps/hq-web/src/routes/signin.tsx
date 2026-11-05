import { createFileRoute } from '@tanstack/react-router'
import { SignedOutView } from '~/components/SignInPrompt'

export const Route = createFileRoute('/signin')({
  component: () => <SignedOutView pathname="/signin" embedded={false} />,
})
