import { createFileRoute } from '@tanstack/react-router'
import { VaultShell } from '~/components/vault/VaultNav'

// Every Vault page sits in the shell: the location bar on top and the folder tree beside the page.
export const Route = createFileRoute('/notes')({
    component: VaultShell,
    ssr: false,
})
