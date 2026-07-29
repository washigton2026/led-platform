import { useEffect, useRef, useState } from 'react'
import { fixture, type Health } from './readmodel'

// Colour-blind-safe status: never colour-only — a symbol + a word carry the meaning too.
const STATUS: Record<Health, { symbol: string; label: string; color: string }> = {
  ok: { symbol: '✓', label: 'OK', color: '#2e7d32' },
  warning: { symbol: '▲', label: 'Atenção', color: '#b26a00' },
  critical: { symbol: '■', label: 'Crítico', color: '#c62828' },
}

export function App() {
  const [health, setHealth] = useState<Health>('ok')
  const rm = fixture(health)
  const cycle = () =>
    setHealth((h) => (h === 'ok' ? 'warning' : h === 'warning' ? 'critical' : 'ok'))

  const s = STATUS[rm.health]

  return (
    <main style={styles.page}>
      <h1 style={{ marginTop: 0 }}>LUMYX — Console (spike React)</h1>

      {/* Live region: a screen reader must announce the status change. */}
      <div
        role="status"
        aria-live="polite"
        style={{ ...styles.banner, borderColor: s.color }}
      >
        <span aria-hidden="true" style={{ fontSize: 22, color: s.color }}>
          {s.symbol}
        </span>
        <strong>Saúde do output: {s.label}</strong>
      </div>

      <button onClick={cycle} style={styles.btn}>
        Ciclar status (testa o anúncio do leitor de tela)
      </button>

      <section aria-labelledby="dev-h">
        <h2 id="dev-h">Controladores</h2>
        <table style={styles.table}>
          <caption style={styles.caption}>Status por controlador</caption>
          <thead>
            <tr>
              <th scope="col">ID</th>
              <th scope="col">Conectado</th>
              <th scope="col">Frames</th>
              <th scope="col">Último envio (ms)</th>
            </tr>
          </thead>
          <tbody>
            {rm.devices.map((d) => (
              <tr key={d.id}>
                <td>{d.id}</td>
                <td>{d.connected ? 'sim' : 'não'}</td>
                <td>{d.frames_sent}</td>
                <td>{d.last_send_ms}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section aria-labelledby="met-h">
        <h2 id="met-h">Métricas</h2>
        <dl style={styles.metrics}>
          <dt>frames</dt><dd>{rm.metrics.frames}</dd>
          <dt>drops</dt><dd>{rm.metrics.drops}</dd>
          <dt>p50 (µs)</dt><dd>{rm.metrics.p50_us}</dd>
          <dt>p99 (µs)</dt><dd>{rm.metrics.p99_us}</dd>
        </dl>
      </section>

      <section aria-labelledby="pv-h">
        <h2 id="pv-h">Preview</h2>
        <Preview />
      </section>
    </main>
  )
}

/**
 * Preview de ~10k pontos. Renderiza em Canvas2D (mensurável em qualquer máquina) e mostra se
 * `navigator.gpu` (WebGPU) está disponível — a rota WebGPU é IDÊNTICA nas duas stacks (é o
 * mesmo `<canvas>` + a mesma API), então este eixo quase não diferencia Leptos de React; a
 * decisão real está em a11y/DX. Troque por um renderer WebGPU aqui se quiser o fps de GPU.
 */
function Preview() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [fps, setFps] = useState(0)
  const gpu = typeof navigator !== 'undefined' && 'gpu' in navigator

  useEffect(() => {
    const cvs = canvasRef.current
    if (!cvs) return
    const ctx = cvs.getContext('2d')!
    const N = 10_000
    const pts = Array.from({ length: N }, () => ({
      x: Math.random(),
      y: Math.random(),
      vx: (Math.random() - 0.5) * 0.002,
      vy: (Math.random() - 0.5) * 0.002,
    }))
    let raf = 0
    let last = performance.now()
    let frames = 0
    const loop = () => {
      const w = cvs.width, h = cvs.height
      ctx.fillStyle = '#0b0b0b'
      ctx.fillRect(0, 0, w, h)
      ctx.fillStyle = '#4fc3f7'
      for (const p of pts) {
        p.x += p.vx; p.y += p.vy
        if (p.x < 0 || p.x > 1) p.vx *= -1
        if (p.y < 0 || p.y > 1) p.vy *= -1
        ctx.fillRect(p.x * w, p.y * h, 2, 2)
      }
      frames++
      const now = performance.now()
      if (now - last >= 500) {
        setFps(Math.round((frames * 1000) / (now - last)))
        frames = 0; last = now
      }
      raf = requestAnimationFrame(loop)
    }
    raf = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(raf)
  }, [])

  return (
    <div>
      <p style={{ margin: '4px 0' }}>
        WebGPU: <strong>{gpu ? 'disponível' : 'indisponível'}</strong> · fps (2D, 10k pts):{' '}
        <strong aria-live="off">{fps}</strong>
      </p>
      <canvas
        ref={canvasRef}
        width={640}
        height={240}
        role="img"
        aria-label="Preview animado de 10 mil pixels"
        style={{ width: '100%', maxWidth: 640, border: '1px solid #444', background: '#0b0b0b' }}
      />
    </div>
  )
}

const styles: Record<string, React.CSSProperties> = {
  page: { maxWidth: 760, margin: '2rem auto', padding: '0 1rem', color: '#eee', background: '#111', fontFamily: 'system-ui, sans-serif', lineHeight: 1.5 },
  banner: { display: 'flex', gap: 10, alignItems: 'center', padding: '10px 14px', border: '2px solid', borderRadius: 8, margin: '12px 0' },
  btn: { padding: '8px 14px', fontSize: 15, cursor: 'pointer', margin: '4px 0 16px' },
  table: { width: '100%', borderCollapse: 'collapse' },
  caption: { textAlign: 'left', fontStyle: 'italic', marginBottom: 6 },
  metrics: { display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '2px 16px', maxWidth: 260 },
}
