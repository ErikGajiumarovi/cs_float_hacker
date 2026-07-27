import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const catalog = {
  schema_version: 'fixture-catalog',
  source: { name: 'Fixture', url: 'https://example.test/catalog', license: 'MIT', retrieved_at: 'fixed', note: 'fixture' },
  skins: [
    { id: 'input', name: 'Input', collection_id: 'test', collection_name: 'Test', rarity: 'classified', min_float: 0, max_float: 1, stattrak_supported: false },
    { id: 'other-rarity', name: 'Other rarity', collection_id: 'test', collection_name: 'Test', rarity: 'restricted', min_float: 0, max_float: 1, stattrak_supported: false },
    { id: 'output', name: 'Output', collection_id: 'test', collection_name: 'Test', rarity: 'covert', min_float: 0, max_float: 1, stattrak_supported: false },
  ],
  listing_count: 0,
  limitations: [],
}

const float32 = { value: 0.1, bits: 1036831949, display: '0.100000001' }
const simulation = {
  valid: true,
  contract_size: 10,
  input_rarity: 'classified',
  average_adjusted: float32,
  outcomes: [{
    skin_id: 'output',
    skin_name: 'Output',
    collection_name: 'Test',
    probability: 1,
    probability_percent: '100.00%',
    predicted_float: float32,
    wear: { name: 'Minimal Wear', distance_to_nearest_boundary: float32, nearest_boundary: float32, near_boundary: false },
  }],
  warnings: [],
}

const partialSimulation = {
  input_rarity: 'classified',
  selected_slots: 1,
  floats_specified: 0,
  remaining_float_slots: 10,
  outcomes: [{
    skin_id: 'output',
    skin_name: 'Output',
    collection_name: 'Test',
    known_inputs_from_collection: 1,
    probability_min: 0.1,
    probability_max: 1,
    predicted_float_min: float32,
    predicted_float_max: float32,
  }],
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

  it('switches to a full ten-item contract simulation', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_catalog') return Promise.resolve(catalog)
      if (command === 'analyze_contract') return Promise.resolve(simulation)
      if (command === 'preview_partial_contract') return Promise.resolve(partialSimulation)
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })

    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: /У меня есть предметы/i }))

    expect(screen.getByRole('heading', { name: 'Что у вас уже есть?' })).toBeInTheDocument()
    expect(screen.getByLabelText('Предмет 1')).toBeInTheDocument()
    expect(screen.getByLabelText('Предмет 10')).toBeInTheDocument()
    expect(screen.getByLabelText('Float предмета 1')).toBeInTheDocument()

    fireEvent.change(screen.getByLabelText('Предмет 1'), { target: { value: 'input' } })
    expect(within(screen.getByLabelText('Предмет 2')).queryByRole('option', { name: /Other rarity/i })).not.toBeInTheDocument()
    expect(await screen.findByText('10.00% — 100.00%')).toBeInTheDocument()
    for (let index = 1; index <= 10; index += 1) {
      fireEvent.change(screen.getByLabelText(`Предмет ${index}`), { target: { value: 'input' } })
      fireEvent.change(screen.getByLabelText(`Float предмета ${index}`), { target: { value: '0.02142857' } })
    }

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('analyze_contract', expect.objectContaining({
      request: expect.objectContaining({ inputs: expect.arrayContaining([expect.objectContaining({ skin_id: 'input', float_value: 0.02142857 })]) }),
    })))
    expect(await screen.findByText('Средний adjusted float:')).toBeInTheDocument()
  })
})
