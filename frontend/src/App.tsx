import { FormEvent, useEffect, useMemo, useState } from 'react'

const API_BASE = import.meta.env.VITE_API_URL ?? 'http://localhost:8080'

type Rarity = 'consumer' | 'industrial' | 'mil_spec' | 'restricted' | 'classified' | 'covert' | 'extraordinary'

type Skin = {
  id: string
  name: string
  collection_id: string
  collection_name: string
  rarity: Rarity
  min_float: number
  max_float: number
  stattrak_supported: boolean
}

type Float32 = { value: number; bits: number; display: string }
type Wear = { name: string; distance_to_nearest_boundary: Float32; nearest_boundary: Float32; near_boundary: boolean }
type Outcome = {
  skin_id: string
  skin_name: string
  collection_name: string
  probability: number
  probability_percent: string
  predicted_float: Float32
  wear: Wear
}

type Catalog = {
  schema_version: string
  source: { name: string; url: string; license: string; retrieved_at: string; note: string }
  skins: Skin[]
  listing_count: number
  limitations: string[]
}

type SyncStatus = {
  running: boolean
  last_success_at: number | null
  last_error: string | null
  imported_skins: number
  imported_collections: number
  caps_verification: { compared: number; matching: number; mismatching: number; not_directly_verifiable: number } | null
}

type MarketStatus = {
  provider: string
  enabled: boolean
  cache_ttl_seconds: number
  minimum_request_interval_ms: number
  cached_searches: number
  max_candidates_per_skin: number
}

type PlannedInput = {
  slot: number
  listing_id: string | null
  source: string
  skin_id: string
  skin_name: string
  float_value: Float32
  adjusted_float: Float32
  price_cents: number | null
  market_url: string | null
  inspect_link: string | null
  owned: boolean
}

type Plan = {
  status: string
  message: string
  target_skin: Skin
  target_adjusted: Float32
  acceptable_output_range: [Float32, Float32]
  contract_size: number
  target_probability: string
  candidate_input_skins: Skin[]
  selected_inputs: PlannedInput[]
  total_price_cents: number
  pricing_available: boolean
  predicted_target_float: Float32
  target_distance: Float32
  target_wear: Wear
  all_outcomes: Outcome[]
  optimizer: string
  warnings: string[]
}

type Simulation = {
  valid: boolean
  contract_size: number
  input_rarity: Rarity
  average_adjusted: Float32
  outcomes: Outcome[]
  warnings: string[]
}

type OwnedInput = { skinId: string; floatValue: string }
type TargetMode = 'exact' | 'maximum' | 'range'

const rarityLabel: Record<Rarity, string> = {
  consumer: 'Consumer',
  industrial: 'Industrial',
  mil_spec: 'Mil-Spec',
  restricted: 'Restricted',
  classified: 'Classified',
  covert: 'Covert',
  extraordinary: 'Extraordinary',
}

function money(cents: number | null) {
  if (cents === null) return '—'
  return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(cents / 100)
}

async function readError(response: Response) {
  try {
    const payload = await response.json() as { error?: string }
    return payload.error ?? `HTTP ${response.status}`
  } catch {
    return `HTTP ${response.status}`
  }
}

function FloatValue({ value, bits = false }: { value: Float32; bits?: boolean }) {
  return <span className="mono" title={`IEEE-754 float32 bits: 0x${value.bits.toString(16).padStart(8, '0')}`}>
    {value.display}{bits && <small> · 0x{value.bits.toString(16).padStart(8, '0')}</small>}
  </span>
}

function OutcomeTable({ outcomes }: { outcomes: Outcome[] }) {
  return <div className="table-wrap">
    <table>
      <thead>
        <tr><th>Исход</th><th>Шанс названия</th><th>Прогноз float32</th><th>Wear</th><th>Граница</th></tr>
      </thead>
      <tbody>
        {outcomes.map((outcome) => <tr key={outcome.skin_id}>
          <td><strong>{outcome.skin_name}</strong><span className="subtle">{outcome.collection_name}</span></td>
          <td>{outcome.probability_percent}</td>
          <td><FloatValue value={outcome.predicted_float} /></td>
          <td>{outcome.wear.name}</td>
          <td className={outcome.wear.near_boundary ? 'boundary-alert' : ''}>
            {outcome.wear.near_boundary ? '⚠ ' : ''}<FloatValue value={outcome.wear.distance_to_nearest_boundary} /> до <FloatValue value={outcome.wear.nearest_boundary} />
          </td>
        </tr>)}
      </tbody>
    </table>
  </div>
}

export default function App() {
  const [catalog, setCatalog] = useState<Catalog | null>(null)
  const [syncStatus, setSyncStatus] = useState<SyncStatus | null>(null)
  const [marketStatus, setMarketStatus] = useState<MarketStatus | null>(null)
  const [targetSkinId, setTargetSkinId] = useState('')
  const [targetMode, setTargetMode] = useState<TargetMode>('exact')
  const [targetValue, setTargetValue] = useState('0.150000')
  const [rangeMin, setRangeMin] = useState('0.149500')
  const [rangeMax, setRangeMax] = useState('0.150500')
  const [delta, setDelta] = useState('0.000100')
  const [priority, setPriority] = useState<'cheapest' | 'closest'>('cheapest')
  const [budget, setBudget] = useState('')
  const [ownedInputs, setOwnedInputs] = useState<OwnedInput[]>([])
  const [plan, setPlan] = useState<Plan | null>(null)
  const [simulation, setSimulation] = useState<Simulation | null>(null)
  const [manualSkinId, setManualSkinId] = useState('')
  const [manualFloat, setManualFloat] = useState('0.02142857')
  const [loading, setLoading] = useState(true)
  const [planning, setPlanning] = useState(false)
  const [simulating, setSimulating] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    fetch(`${API_BASE}/api/catalog`)
      .then(async response => {
        if (!response.ok) throw new Error(await readError(response))
        return response.json() as Promise<Catalog>
      })
      .then(data => {
        setCatalog(data)
        const firstTarget = data.skins.find(skin => skin.id === 'awp-dragon-lore') ?? data.skins.find(skin => skin.rarity === 'covert')
        if (firstTarget) setTargetSkinId(firstTarget.id)
      })
      .catch(problem => setError(`Не удалось загрузить API: ${problem.message}`))
      .finally(() => setLoading(false))
  }, [])

  useEffect(() => {
    fetch(`${API_BASE}/api/market-status`)
      .then(response => response.ok ? response.json() as Promise<MarketStatus> : null)
      .then(status => { if (status) setMarketStatus(status) })
      .catch(() => undefined)
  }, [])

  useEffect(() => {
    fetch(`${API_BASE}/api/sync-status`)
      .then(response => response.ok ? response.json() as Promise<SyncStatus> : null)
      .then(status => { if (status) setSyncStatus(status) })
      .catch(() => undefined)
  }, [])

  const target = useMemo(() => catalog?.skins.find(skin => skin.id === targetSkinId) ?? null, [catalog, targetSkinId])
  const targetChoices = useMemo(() => catalog?.skins.filter(skin => skin.rarity === 'covert') ?? [], [catalog])
  const inputChoices = useMemo(() => {
    if (!catalog || !target) return []
    return catalog.skins.filter(skin => skin.collection_id === target.collection_id && skin.rarity === 'classified')
  }, [catalog, target])

  useEffect(() => {
    if (inputChoices.length > 0) setManualSkinId(inputChoices[0].id)
  }, [targetSkinId])

  function addOwned() {
    if (!inputChoices[0]) return
    setOwnedInputs(current => [...current, { skinId: inputChoices[0].id, floatValue: '0.02142857' }])
  }

  function updateOwned(index: number, patch: Partial<OwnedInput>) {
    setOwnedInputs(current => current.map((input, position) => position === index ? { ...input, ...patch } : input))
  }

  function removeOwned(index: number) {
    setOwnedInputs(current => current.filter((_, position) => position !== index))
  }

  async function requestPlan(event: FormEvent) {
    event.preventDefault()
    if (!target) return
    setPlanning(true)
    setError(null)
    setSimulation(null)
    const targetPayload = targetMode === 'range'
      ? { mode: 'range', min: Number(rangeMin), max: Number(rangeMax) }
      : { mode: targetMode, value: Number(targetValue) }
    const payload = {
      target_skin_id: target.id,
      target: targetPayload,
      delta: Number(delta),
      priority,
      budget_cents: budget.trim() ? Math.round(Number(budget) * 100) : undefined,
      owned_inputs: ownedInputs.map(input => ({ skin_id: input.skinId, float_value: Number(input.floatValue) })),
    }
    try {
      const response = await fetch(`${API_BASE}/api/plan`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      })
      if (!response.ok) throw new Error(await readError(response))
      setPlan(await response.json() as Plan)
    } catch (problem) {
      setPlan(null)
      setError(problem instanceof Error ? problem.message : 'Не удалось рассчитать план')
    } finally {
      setPlanning(false)
    }
  }

  async function simulate(inputs?: Array<{ skin_id: string; float_value: number }>) {
    const actualInputs = inputs ?? Array.from({ length: 10 }, () => ({ skin_id: manualSkinId, float_value: Number(manualFloat) }))
    if (actualInputs.some(input => !input.skin_id || !Number.isFinite(input.float_value))) {
      setError('Для симуляции нужны skin и конечное float-значение.')
      return
    }
    setSimulating(true)
    setError(null)
    try {
      const response = await fetch(`${API_BASE}/api/analyze`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ contract_size: 10, stattrak: false, inputs: actualInputs }),
      })
      if (!response.ok) throw new Error(await readError(response))
      setSimulation(await response.json() as Simulation)
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Не удалось проверить контракт')
    } finally {
      setSimulating(false)
    }
  }

  if (loading) return <main className="loading"><div className="orbit" />Загрузка каталога и float32-ядра…</main>

  return <main>
    <header className="hero">
      <div className="hero-inner">
        <div>
          <p className="eyebrow">MVP · reverse trade-up planner</p>
          <h1>Float<span>craft</span></h1>
          <p className="lede">От желаемого CS2 float — к проверяемому контракту. Вероятность скина, float и стоимость показаны отдельно.</p>
        </div>
        <div className="engine-card"><span className="pulse" /> Backend computes <strong>IEEE‑754 float32</strong><small>Не JavaScript number</small></div>
      </div>
    </header>

    <section className="container notice">
      <strong>Каталог и математика:</strong> после успешной синхронизации планировщик переключается на полный versioned snapshot ByMykel. Лоты появляются только через provider с подтверждённым exact float; встроенный MVP-пул остаётся тестовым fallback.
      {syncStatus && <span className="sync-line">Catalog sync: {syncStatus.running ? 'обновление…' : syncStatus.last_error ? `ошибка — ${syncStatus.last_error}` : syncStatus.last_success_at ? `${syncStatus.imported_skins} skins / ${syncStatus.imported_collections} collections; direct caps: ${syncStatus.caps_verification?.matching ?? 0} match, ${syncStatus.caps_verification?.mismatching ?? 0} mismatch, ${syncStatus.caps_verification?.not_directly_verifiable ?? 0} review` : 'ожидание первого обновления'}</span>}
      {marketStatus && <span className="sync-line">Market: {marketStatus.enabled ? `${marketStatus.provider}, cache ${marketStatus.cache_ttl_seconds}s, ≤${marketStatus.max_candidates_per_skin} candidates/skin` : 'provider не настроен — будет показан ideal_math'}</span>}
    </section>

    {error && <section className="container error"><strong>Расчёт остановлен:</strong> {error}</section>}

    <section className="container workspace">
      <form className="planner card" onSubmit={requestPlan}>
        <div className="section-heading"><div><p className="eyebrow">01 / target</p><h2>Что хотим получить?</h2></div><span className="tag">10 Classified → Covert</span></div>
        <label>Целевой skin
          <select value={targetSkinId} onChange={event => { setTargetSkinId(event.target.value); setPlan(null) }}>
            {targetChoices.map(skin => <option key={skin.id} value={skin.id}>{skin.name} · {skin.collection_name}</option>)}
          </select>
        </label>
        {target && <div className="cap-row"><span>Cap</span><span className="mono">{target.min_float.toFixed(9)}</span> — <span className="mono">{target.max_float.toFixed(9)}</span><span>· {rarityLabel[target.rarity]}</span></div>}

        <div className="form-grid three">
          <label>Тип цели
            <select value={targetMode} onChange={event => setTargetMode(event.target.value as TargetMode)}>
              <option value="exact">Точное значение</option>
              <option value="maximum">Не выше</option>
              <option value="range">Диапазон</option>
            </select>
          </label>
          {targetMode === 'range' ? <>
            <label>Min float<input inputMode="decimal" value={rangeMin} onChange={event => setRangeMin(event.target.value)} /></label>
            <label>Max float<input inputMode="decimal" value={rangeMax} onChange={event => setRangeMax(event.target.value)} /></label>
          </> : <label>Target float<input inputMode="decimal" value={targetValue} onChange={event => setTargetValue(event.target.value)} /></label>}
          <label>δ результата<input inputMode="decimal" value={delta} onChange={event => setDelta(event.target.value)} /></label>
        </div>

        <div className="section-heading compact"><div><p className="eyebrow">02 / owned inventory</p><h3>Уже есть {ownedInputs.length} / 10</h3></div><button type="button" className="text-button" onClick={addOwned} disabled={!inputChoices.length || ownedInputs.length >= 10}>+ Добавить предмет</button></div>
        {ownedInputs.length === 0 ? <p className="empty">Можно оставить пустым или добавить, например, 5 своих M4A1‑S | Knight с exact float.</p> : <div className="owned-list">
          {ownedInputs.map((input, index) => <div className="owned-row" key={index}>
            <span className="slot">{index + 1}</span>
            <select value={input.skinId} onChange={event => updateOwned(index, { skinId: event.target.value })}>
              {inputChoices.map(skin => <option value={skin.id} key={skin.id}>{skin.name}</option>)}
            </select>
            <input aria-label={`Float owned item ${index + 1}`} inputMode="decimal" value={input.floatValue} onChange={event => updateOwned(index, { floatValue: event.target.value })} />
            <button type="button" className="icon-button" onClick={() => removeOwned(index)} aria-label="Удалить предмет">×</button>
          </div>)}
        </div>}

        <div className="form-grid two lower-fields">
          <label>Приоритет
            <select value={priority} onChange={event => setPriority(event.target.value as 'cheapest' | 'closest')}>
              <option value="cheapest">Самый дешёвый в цели</option>
              <option value="closest">Самый близкий float</option>
            </select>
          </label>
          <label>Бюджет, USD <span className="optional">optional</span><input inputMode="decimal" placeholder="например, 55000" value={budget} onChange={event => setBudget(event.target.value)} /></label>
        </div>
        <button className="primary" disabled={planning || !target}>{planning ? 'Подбираю комбинацию…' : 'Построить проверяемый план →'}</button>
      </form>

      <aside className="math-panel card">
        <p className="eyebrow">float mapping</p>
        <h2>«Absolute» float<br />не отдельная БД</h2>
        <p>Он вычисляется из cap конкретного skin на универсальной шкале 0–1.</p>
        <code>adjusted = (raw − min) / (max − min)</code>
        <code>out = out_min + avg(adjusted) × span</code>
        <hr />
        <p className="subtle">Источник MVP-caps: <a href={catalog?.source.url} target="_blank" rel="noreferrer">{catalog?.source.name}</a> · {catalog?.source.license} · snapshot {catalog?.schema_version}</p>
        <p className="subtle">Формула и последовательное float32-округление сверены вручную с FloatJitsu. UI лишь показывает ответ API.</p>
      </aside>
    </section>

    {plan && <section className="container results">
      <div className="result-banner">
        <span className={`status ${plan.status}`}>{plan.status.replace('_', ' ')}</span>
        <div><strong>{plan.message}</strong><p>{plan.optimizer}</p></div>
        <button className="secondary" onClick={() => simulate(plan.selected_inputs.map(item => ({ skin_id: item.skin_id, float_value: item.float_value.value })))} disabled={simulating}>{simulating ? 'Проверяю…' : 'Проверить набор'}</button>
      </div>
      <div className="metric-grid">
        <div className="metric"><span>Target adjusted</span><strong><FloatValue value={plan.target_adjusted} /></strong><small>нужно среднее</small></div>
        <div className="metric"><span>Прогноз target float</span><strong><FloatValue value={plan.predicted_target_float} /></strong><small>{plan.target_wear.name}</small></div>
        <div className="metric"><span>Шанс названия</span><strong>{plan.target_probability}</strong><small>не зависит от float</small></div>
        <div className="metric"><span>Цена недостающих</span><strong>{plan.pricing_available ? money(plan.total_price_cents) : '—'}</strong><small>{plan.pricing_available ? 'проверенные кандидаты' : 'лоты не подключены'}</small></div>
      </div>

      <div className="card result-card">
        <div className="section-heading"><div><p className="eyebrow">selected inputs</p><h2>10 слотов, сохранённый порядок</h2></div><span className="subtle">Допустимый output: <FloatValue value={plan.acceptable_output_range[0]} /> — <FloatValue value={plan.acceptable_output_range[1]} /></span></div>
        <div className="table-wrap"><table>
          <thead><tr><th>Slot</th><th>Предмет</th><th>Exact float</th><th>Adjusted</th><th>Цена</th><th>Источник</th></tr></thead>
          <tbody>{plan.selected_inputs.map(input => <tr key={`${input.slot}-${input.listing_id ?? 'owned'}`}>
            <td><span className="slot">{input.slot}</span></td>
            <td><strong>{input.skin_name}</strong></td>
            <td><FloatValue value={input.float_value} bits /></td>
            <td><FloatValue value={input.adjusted_float} /></td>
            <td>{input.owned ? 'ваш предмет' : money(input.price_cents)}</td>
            <td>{input.market_url ? <><a href={input.market_url} target="_blank" rel="noreferrer">{input.source} ↗</a>{input.inspect_link && <> · <a href={input.inspect_link}>inspect ↗</a></>}</> : input.source}</td>
          </tr>)}</tbody>
        </table></div>
      </div>
      <div className="card result-card"><div className="section-heading"><div><p className="eyebrow">all outcomes</p><h2>Не смешиваем шанс и float</h2></div></div><OutcomeTable outcomes={plan.all_outcomes} /></div>
      <Warnings messages={plan.warnings} />
    </section>}

    <section className="container simulator card">
      <div className="section-heading"><div><p className="eyebrow">forward validation</p><h2>Проверить ручной контракт</h2><p>Десять одинаковых входов — быстрый способ проверить float32-формулу и wear boundary.</p></div><span className="tag">manual</span></div>
      <div className="manual-controls">
        <label>Input skin<select value={manualSkinId} onChange={event => setManualSkinId(event.target.value)}>{inputChoices.map(skin => <option value={skin.id} key={skin.id}>{skin.name}</option>)}</select></label>
        <label>Exact float<input inputMode="decimal" value={manualFloat} onChange={event => setManualFloat(event.target.value)} /></label>
        <button className="secondary" onClick={() => simulate()} disabled={simulating || !manualSkinId}>{simulating ? 'Считаю…' : 'Симулировать 10 ×'}</button>
      </div>
      {simulation && <div className="simulation-output"><div className="average">avg adjusted: <FloatValue value={simulation.average_adjusted} bits /></div><OutcomeTable outcomes={simulation.outcomes} /><Warnings messages={simulation.warnings} /></div>}
    </section>

    <section className="container limitations card"><p className="eyebrow">границы данных</p><h2>Что требует внешнего источника</h2><ul>{catalog?.limitations.map(item => <li key={item}>{item}</li>)}</ul><p>Полный catalog, float32-расчёт и reverse path работают локально. Для конкретных покупок всё ещё необходим provider, который законно отдаёт listing, inspect payload и exact float.</p></section>
    <footer className="container footer">Floatcraft MVP · Rust API + React/TypeScript · catalog snapshot {catalog?.schema_version}</footer>
  </main>
}

function Warnings({ messages }: { messages: string[] }) {
  return <div className="warnings">{messages.map(message => <p key={message}>⚑ {message}</p>)}</div>
}
