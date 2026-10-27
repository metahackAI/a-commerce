import { usd } from "../api";

export type SalesData = {
  day: string; revenue: number; orders: number; usual_revenue: number; usual_orders: number; difference: number;
  difference_pct: number; baseline: string; next_steps: string[];
  causes: { cause: string; detail: string | null; evidence: string; estimated: number }[];
  history: { day: string; revenue: number; orders: number }[];
};

const W = 640, H = 170, PAD = 28;

function Chart({ data }: { data: SalesData }) {
  const vals = data.history.map((h) => h.revenue);
  const lo = Math.min(...vals, data.usual_revenue) * 0.92, hi = Math.max(...vals, data.usual_revenue) * 1.04;
  const x = (i: number) => PAD + (i * (W - PAD * 2)) / (vals.length - 1);
  const y = (v: number) => H - PAD - ((v - lo) / (hi - lo)) * (H - PAD * 2);
  const last = vals.length - 1;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Daily revenue, last 14 days" style={{ width: "100%", height: "auto" }}>
      <line x1={PAD} x2={W - PAD} y1={y(data.usual_revenue)} y2={y(data.usual_revenue)} stroke="var(--mute)" strokeDasharray="4 4" />
      <text x={W - PAD} y={y(data.usual_revenue) - 6} textAnchor="end" fontSize="11" fill="var(--mute)">
        Usual: {usd(data.usual_revenue)}
      </text>
      <polyline fill="none" stroke="var(--ink)" strokeWidth="1.8" points={vals.map((v, i) => `${x(i)},${y(v)}`).join(" ")} />
      <circle cx={x(last)} cy={y(vals[last])} r="4" fill="var(--ink)" />
      <text x={x(last) - 8} y={y(vals[last]) + 18} textAnchor="end" fontSize="11.5" fontWeight="600" fill="var(--ink)">
        {new Date(data.day + "T12:00").toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" })} · {usd(data.revenue)}
      </text>
      {data.history.map((h, i) => (i % 3 === 0 || i === last) && (
        <text key={h.day} x={x(i)} y={H - 6} textAnchor="middle" fontSize="10.5" fill="var(--mute)">{h.day.slice(5).replace("-", "/")}</text>
      ))}
    </svg>
  );
}

export function SalesExplainer({ data, busy, onAsk }: { data: SalesData; busy: boolean; onAsk: (request: string) => void }) {
  const dayLabel = new Date(data.day + "T12:00").toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" });
  return (
    <section className="cs" data-testid="sales-explainer">
      <div className="csh"><span className="id">Read only</span><h1>Sales on {dayLabel}</h1><span className="tag">Nothing changed</span></div>
      <div className="body">
        <div className="intent" style={{ gridTemplateColumns: "repeat(3,1fr)" }}>
          <div><small>Revenue, {dayLabel}</small><b>{usd(data.revenue)}</b><span>Usual {usd(data.usual_revenue)}</span></div>
          <div><small>Difference</small><b>{data.difference < 0 ? "−" : "+"}{usd(Math.abs(data.difference))} ({data.difference_pct}%)</b><span>vs {data.baseline}</span></div>
          <div><small>Orders</small><b>{data.orders}</b><span>usual {data.usual_orders}</span></div>
        </div>
        <div><h2>Daily revenue, last 14 days</h2><div className="grp" style={{ padding: "8px 6px" }}><Chart data={data} /></div></div>
        <div>
          <h2>What explains it<span className="r">estimates from {data.baseline}, not exact</span></h2>
          <div className="grp">
            <table>
              <thead><tr><th>Cause</th><th>Evidence</th><th className="n">Estimated</th></tr></thead>
              <tbody>
                {data.causes.map((c) => (
                  <tr key={c.cause}>
                    <td><b style={{ fontWeight: 600 }}>{c.cause}</b>{c.detail && <small className="mute" style={{ display: "block" }}>{c.detail}</small>}</td>
                    <td>{c.evidence}</td>
                    <td className="n">{c.estimated < 0 ? "−" : "+"}{usd(Math.abs(c.estimated))}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
        <div>
          <h2>What I can do next<span className="r">each becomes a change set for you to approve</span></h2>
          <div className="ua" style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            {data.next_steps.map((s) => <button key={s} className="btn sm" disabled={busy} onClick={() => onAsk(s)}>{s}</button>)}
          </div>
        </div>
      </div>
    </section>
  );
}
