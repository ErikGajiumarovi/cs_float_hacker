import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'

export type AppUpdate = {
  version: string
  body?: string
  install: () => Promise<void>
}

export async function checkForUpdate(): Promise<AppUpdate | null> {
  const update = await check()
  if (!update) return null
  return {
    version: update.version,
    body: update.body,
    install: async () => {
      await update.downloadAndInstall()
      await relaunch()
    },
  }
}
