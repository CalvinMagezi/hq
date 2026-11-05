import { createFileRoute } from '@tanstack/react-router'
import { SignInPage } from '~/components/SignInPrompt'

export const Route = createFileRoute('/signin')({
  component: SignInPage,
})
