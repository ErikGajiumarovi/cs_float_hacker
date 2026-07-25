import { invoke } from '@tauri-apps/api/core'

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args)
}

export const backend = {
  catalog: <T>() => call<T>('get_catalog'),
  syncStatus: <T>() => call<T>('get_sync_status'),
  syncNow: <T>() => call<T>('sync_catalog_now'),
  marketStatus: <T>() => call<T>('get_market_status'),
  regressionStatus: <T>() => call<T>('get_regression_status'),
  analyze: <T>(request: unknown) => call<T>('analyze_contract', { request }),
  plan: <T>(request: unknown) => call<T>('plan_contract', { request }),
  recordContract: <T>(submission: unknown) => call<T>('record_contract', { submission }),
  settings: <T>() => call<T>('get_settings'),
  setMarketEnabled: <T>(enabled: boolean) => call<T>('set_market_enabled', { enabled }),
  acknowledgeLiveMarketNotice: <T>() => call<T>('acknowledge_live_market_notice'),
}
