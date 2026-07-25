import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const inputSkin = {
  id: 'collection-test/input',
  name: 'Input',
  collection_id: 'collection-test',
  collection_name: 'Test',
  rarity: 'classified',
  min_float: 0,
  max_float: 1,
  stattrak_supported: false,
}

const outputSkin = {
  id: 'collection-test/output',
  name: 'Output',
  collection_id: 'collection-test',
  collection_name: 'Test',
  rarity: 'covert',
  min_float: 0,
  max_float: 1,
  stattrak_supported: false,
}

function float32(value: number) {
  const bytes = new ArrayBuffer(4)
  const view = new DataView(bytes)
  view.setFloat32(0, value, true)
  return { value, bits: view.getUint32(0, true), display: value.toFixed(9) }
}

const predicted = float32(0.1)

const catalog = {
  schema_version: 'fixture-catalog',
  source: { name: 'Fixture', url: 'https://example.test/catalog', license: 'MIT', retrieved_at: 'fixed', note: 'fixture' },
  skins: [inputSkin, outputSkin],
  listing_count: 0,
  limitations: [],
}

const analysis = {
  valid: true,
  contract_size: 10,
  input_rarity: 'classified',
  average_adjusted: predicted,
  outcomes: [{
    skin_id: outputSkin.id,
    skin_name: outputSkin.name,
    collection_name: outputSkin.collection_name,
    probability: 1,
    probability_percent: '100.00%',
    predicted_float: predicted,
    wear: { name: 'Minimal Wear', distance_to_nearest_boundary: float32(0.03), nearest_boundary: float32(0.07), near_boundary: false },
  }],
  warnings: [],
}

afterEach(() => {
  invokeMock.mockReset()
})

describe('regression collector', () => {
  it('previews a complete contract before saving it and refreshes deterministic progress', async () => {
    let saved = false
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_catalog') return Promise.resolve(catalog)
      if (command === 'get_market_status') return Promise.resolve({ provider: 'disabled', enabled: false, cache_ttl_seconds: 0, minimum_request_interval_ms: 0, cached_searches: 0, max_candidates_per_skin: 200 })
      if (command === 'get_sync_status') return Promise.resolve({ running: false, last_success_at: null, last_error: null, imported_skins: 0, imported_collections: 0, caps_verification: null })
      if (command === 'get_regression_status') return Promise.resolve({ count: saved ? 1 : 0, evidence_backed_records: saved ? 1 : 0, evidence_backed_exact_matches: saved ? 1 : 0, records_missing_evidence: 0, exact_matches: saved ? 1 : 0, mismatches: 0, ready_for_target: false, target_count: 20 })
      if (command === 'analyze_contract') return Promise.resolve(analysis)
      if (command === 'record_contract') {
        saved = true
        return Promise.resolve({ id: 'contract-fixture', predicted_float: predicted, actual_output_float: predicted, exact_float32_match: true, absolute_difference: float32(0), catalog_schema_version: 'fixture-catalog' })
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })

    render(<App />)

    expect(await screen.findByText('Набор фактических контрактов')).toBeInTheDocument()
    expect(screen.getByText('0 / 20')).toBeInTheDocument()

    for (let index = 1; index <= 10; index += 1) {
      fireEvent.change(screen.getByLabelText(`Input skin ${index}`), { target: { value: inputSkin.id } })
      fireEvent.change(screen.getByLabelText(`Input float ${index}`), { target: { value: '0.1' } })
    }
    fireEvent.change(screen.getByLabelText('Фактический output'), { target: { value: outputSkin.id } })
    fireEvent.change(screen.getByPlaceholderText('например, 0.150000006'), { target: { value: '0.1' } })
    fireEvent.change(screen.getByLabelText('Публичная evidence URL'), { target: { value: 'https://example.test/evidence' } })

    fireEvent.click(screen.getByRole('button', { name: 'Проверить перед сохранением' }))
    expect(await screen.findByText('bit-exact совпадение')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Сохранить evidence-backed запись' })).toBeEnabled()

    fireEvent.click(screen.getByRole('button', { name: 'Сохранить evidence-backed запись' }))
    expect(await screen.findByText(/Сохранено: bit-exact совпадение/)).toBeInTheDocument()
    await waitFor(() => expect(screen.getByText('1 / 20')).toBeInTheDocument())

    const previewRequest = invokeMock.mock.calls.find(([command]) => command === 'analyze_contract')
    expect(previewRequest?.[1]?.request).toMatchObject({
      contract_size: 10,
      stattrak: false,
      inputs: Array.from({ length: 10 }, () => ({ skin_id: inputSkin.id, float_value: 0.1 })),
    })
  })
})
