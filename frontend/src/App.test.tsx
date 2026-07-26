import { render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const catalog = {
  schema_version: 'fixture-catalog',
  source: { name: 'Fixture', url: 'https://example.test/catalog', license: 'MIT', retrieved_at: 'fixed', note: 'fixture' },
  skins: [
    { id: 'input', name: 'Input', collection_id: 'test', collection_name: 'Test', rarity: 'classified', min_float: 0, max_float: 1, stattrak_supported: false },
    { id: 'output', name: 'Output', collection_id: 'test', collection_name: 'Test', rarity: 'covert', min_float: 0, max_float: 1, stattrak_supported: false },
  ],
  listing_count: 0,
  limitations: [],
}

afterEach(() => invokeMock.mockReset())

describe('customer interface', () => {
  it('keeps developer implementation details out of the planner', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_catalog') return Promise.resolve(catalog)
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })

    render(<App />)

    expect(await screen.findByRole('heading', { name: 'Что хотите получить?' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Подобрать предметы →' })).toBeInTheDocument()
    expect(screen.queryByText(/Native Rust/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/IEEE-754/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/reverse trade-up planner/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/catalog snapshot/i)).not.toBeInTheDocument()
    expect(screen.queryByText('Набор фактических контрактов')).not.toBeInTheDocument()
  })
})
