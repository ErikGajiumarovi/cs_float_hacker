import { FormEvent, useEffect, useMemo, useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { backend, isTauri } from './backend'
import { AppUpdate, checkForUpdate } from './updates'

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

type DesktopSettings = {
  marketEnabled: boolean
  liveMarketNoticeAcknowledged: boolean
}

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

function FloatValue({ value }: { value: Float32 }) {
  return <span className="mono">{value.display}</span>
}

function ExternalLink({ href, children }: { href: string; children: React.ReactNode }) {
  function openExternally(event: React.MouseEvent<HTMLAnchorElement>) {
    if (!isTauri) return
    event.preventDefault()
    let protocol = ''
    try { protocol = new URL(href).protocol } catch { return }
    if (protocol === 'https:' || protocol === 'http:' || protocol === 'steam:') void openUrl(href)
  }

  return <a href={href} target="_blank" rel="noreferrer" onClick={openExternally}>{children}</a>
}

function OutcomeTable({ outcomes }: { outcomes: Outcome[] }) {
  return <div className="table-wrap">
    <table>
      <thead>
        <tr><th>Исход</th><th>Шанс</th><th>Прогноз float</th><th>Состояние</th><th>Ближайшая граница</th></tr>
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

function planSummary(status: string) {
  return status === 'closest'
    ? 'В указанном диапазоне нет подходящего набора; показан ближайший вариант.'
    : 'Подходящий набор найден.'
}

export default function App() {
  const [catalog, setCatalog] = useState<Catalog | null>(null)
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
  const [settings, setSettings] = useState<DesktopSettings | null>(null)
  const [availableUpdate, setAvailableUpdate] = useState<AppUpdate | null>(null)
  const [installingUpdate, setInstallingUpdate] = useState(false)

  useEffect(() => {
    backend.catalog<Catalog>()
      .then(data => {
        setCatalog(data)
        const firstTarget = data.skins.find(skin => skin.id === 'awp-dragon-lore') ?? data.skins.find(skin => skin.rarity === 'covert')
        if (firstTarget) setTargetSkinId(firstTarget.id)
      })
      .catch(problem => setError(`Не удалось загрузить локальное ядро: ${problem.message}`))
      .finally(() => setLoading(false))
  }, [])

  useEffect(() => {
    if (!isTauri) return
    void backend.settings<DesktopSettings>().then(setSettings).catch(() => undefined)
    void checkForUpdate().then(setAvailableUpdate).catch(() => undefined)
  }, [])

  const target = useMemo(() => catalog?.skins.find(skin => skin.id === targetSkinId) ?? null, [catalog, targetSkinId])
  const targetChoices = useMemo(() => catalog?.skins.filter(skin => skin.rarity === 'covert') ?? [], [catalog])
  const inputChoices = useMemo(() => {
    if (!catalog || !target) return []
    return catalog.skins.filter(skin => skin.collection_id === target.collection_id && skin.rarity === 'classified')
  }, [catalog, target])
  const collectorSkinChoices = useMemo(() => catalog?.skins
    .filter(skin => skin.rarity !== 'covert' && skin.rarity !== 'extraordinary')
    .sort((left, right) => `${left.name} ${left.collection_name}`.localeCompare(`${right.name} ${right.collection_name}`)) ?? [], [catalog])

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
      setPlan(await backend.plan<Plan>(payload))
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
      setSimulation(await backend.analyze<Simulation>({ contract_size: 10, stattrak: false, inputs: actualInputs }))
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Не удалось проверить контракт')
    } finally {
      setSimulating(false)
    }
  }

  async function toggleMarket(enabled: boolean) {
    try {
      setSettings(await backend.setMarketEnabled<DesktopSettings>(enabled))
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Не удалось изменить настройки Market')
    }
  }

  async function acknowledgeMarketNotice() {
    try {
      setSettings(await backend.acknowledgeLiveMarketNotice<DesktopSettings>())
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Не удалось включить Market')
    }
  }

  async function installUpdate() {
    if (!availableUpdate) return
    setInstallingUpdate(true)
    try {
      await availableUpdate.install()
    } catch (problem) {
      setInstallingUpdate(false)
      setError(problem instanceof Error ? problem.message : 'Не удалось установить обновление')
    }
  }

  if (loading) return <main className="loading"><div className="orbit" />Загрузка…</main>

  return <main>
    <header className="hero">
      <div className="hero-inner">
        <div>
          <h1>Float<span>craft</span></h1>
        </div>
      </div>
    </header>

    {isTauri && settings && <section className="container desktop-banner">
      <label className="market-toggle"><input type="checkbox" checked={settings.marketEnabled} onChange={event => void toggleMarket(event.target.checked)} /> Получать live-лоты Steam Market</label>
      {settings.marketEnabled && !settings.liveMarketNoticeAcknowledged && <div className="market-disclosure">
        <p><strong>Перед первым запросом:</strong> приложение использует публичные предложения Steam Market для подбора предметов. Доступность и цены могут меняться.</p>
        <button className="secondary" type="button" onClick={() => void acknowledgeMarketNotice()}>Понимаю, включить live Market</button>
      </div>}
    </section>}

    {isTauri && availableUpdate && <section className="container desktop-banner update-banner">
      <div><strong>Доступна Floatcraft {availableUpdate.version}</strong>{availableUpdate.body && <p>{availableUpdate.body}</p>}</div>
      <button className="primary" type="button" disabled={installingUpdate} onClick={() => void installUpdate()}>{installingUpdate ? 'Устанавливаю…' : 'Скачать и перезапустить'}</button>
    </section>}

    {error && <section className="container error"><strong>Расчёт остановлен:</strong> {error}</section>}

    <section className="container workspace">
      <form className="planner card" onSubmit={requestPlan}>
        <div className="section-heading"><div><h2>Что хотите получить?</h2></div><span className="tag">Контракт из 10 предметов</span></div>
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

        <div className="section-heading compact"><div><h3>Уже есть {ownedInputs.length} / 10</h3></div><button type="button" className="text-button" onClick={addOwned} disabled={!inputChoices.length || ownedInputs.length >= 10}>+ Добавить предмет</button></div>
        {ownedInputs.length === 0 ? <p className="empty">Можно оставить пустым или добавить свои предметы.</p> : <div className="owned-list">
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
        <button className="primary" disabled={planning || !target}>{planning ? 'Подбираю комбинацию…' : 'Подобрать предметы →'}</button>
      </form>

    </section>

    {plan && <section className="container results">
      <div className="result-banner">
        <span className={`status ${plan.status}`}>{plan.status === 'closest' ? 'ближайший вариант' : 'подходит'}</span>
        <div><strong>{planSummary(plan.status)}</strong></div>
        <button className="secondary" onClick={() => simulate(plan.selected_inputs.map(item => ({ skin_id: item.skin_id, float_value: item.float_value.value })))} disabled={simulating}>{simulating ? 'Проверяю…' : 'Проверить набор'}</button>
      </div>
      <div className="metric-grid">
        <div className="metric"><span>Прогноз target float</span><strong><FloatValue value={plan.predicted_target_float} /></strong><small>{plan.target_wear.name}</small></div>
        <div className="metric"><span>Шанс названия</span><strong>{plan.target_probability}</strong><small>не зависит от float</small></div>
        <div className="metric"><span>Цена недостающих</span><strong>{plan.pricing_available ? money(plan.total_price_cents) : '—'}</strong><small>{plan.pricing_available ? 'проверенные кандидаты' : 'лоты не подключены'}</small></div>
      </div>

      <div className="card result-card">
        <div className="section-heading"><div><h2>Предметы для контракта</h2></div><span className="subtle">Допустимый результат: <FloatValue value={plan.acceptable_output_range[0]} /> — <FloatValue value={plan.acceptable_output_range[1]} /></span></div>
        <div className="table-wrap"><table>
          <thead><tr><th>№</th><th>Предмет</th><th>Float</th><th>Цена</th><th>Ссылка</th></tr></thead>
          <tbody>{plan.selected_inputs.map(input => <tr key={`${input.slot}-${input.listing_id ?? 'owned'}`}>
            <td><span className="slot">{input.slot}</span></td>
            <td><strong>{input.skin_name}</strong></td>
            <td><FloatValue value={input.float_value} /></td>
            <td>{input.owned ? 'ваш предмет' : money(input.price_cents)}</td>
            <td>{input.market_url ? <><ExternalLink href={input.market_url}>Открыть ↗</ExternalLink>{input.inspect_link && <> · <ExternalLink href={input.inspect_link}>Проверить ↗</ExternalLink></>}</> : '—'}</td>
          </tr>)}</tbody>
        </table></div>
      </div>
      <div className="card result-card"><div className="section-heading"><div><h2>Возможные результаты</h2></div></div><OutcomeTable outcomes={plan.all_outcomes} /></div>
    </section>}

    <section className="container simulator card">
      <div className="section-heading"><div><h2>Проверить контракт</h2><p>Укажите предмет и float, чтобы посмотреть возможные результаты контракта.</p></div></div>
      <div className="manual-controls">
        <label>Предмет<select value={manualSkinId} onChange={event => setManualSkinId(event.target.value)}>{inputChoices.map(skin => <option value={skin.id} key={skin.id}>{skin.name}</option>)}</select></label>
        <label>Float<input inputMode="decimal" value={manualFloat} onChange={event => setManualFloat(event.target.value)} /></label>
        <button className="secondary" onClick={() => simulate()} disabled={simulating || !manualSkinId}>{simulating ? 'Считаю…' : 'Симулировать 10 ×'}</button>
      </div>
      {simulation && <div className="simulation-output"><OutcomeTable outcomes={simulation.outcomes} /></div>}
    </section>

    <footer className="container footer">Floatcraft</footer>
  </main>
}
