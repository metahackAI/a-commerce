import type { RefundView } from "../api";
import { usd } from "../api";

const SUGGESTED: Record<string, [string, string]> = { refund: ["Refund", "ok"], ask_for_photos: ["Ask for photos", "warn"], decline: ["Decline", ""] };

export function RefundReview({ view, busy, onApprove }: { view: RefundView; busy: boolean; onApprove: () => void }) {
  const s = view.summary;
  const toLook = view.checks_total - view.checks_passed;
  const status = view.status === "live" ? ["Done · verified", "ok"] : view.status === "waiting_for_approval" ? ["Needs your approval", "warn"] : [view.status, ""];
  return (
    <section className="cs" data-testid="refund-review">
      <div className="csh">
        <span className="id">{s.waiting} requests</span>
        <h1>{view.title}</h1>
        <span className={`tag ${status[1]}`}>{status[0]}</span>
      </div>
      <div className="body">
        <div>
          <h2>What the agent found</h2>
          <div className="intent" style={{ gridTemplateColumns: "repeat(4,1fr)" }}>
            <div><small>Waiting</small><b>{s.waiting} requests</b><span>all over $200</span></div>
            <div><small>Refund</small><b>{s.refund.count} · {usd(s.refund.amount)}</b><span>policy allows</span></div>
            <div><small>Ask for photos</small><b>{s.ask_for_photos}</b><span>damage claims, no photo</span></div>
            <div><small>Decline</small><b>{s.decline}</b><span>outside 30 days</span></div>
          </div>
        </div>
        <div>
          <h2>Requests<span className="r">sorted by what the policy says</span></h2>
          <div className="grp">
            <table>
              <thead><tr><th>Order</th><th>Item</th><th className="n">Amount</th><th>Reason</th><th>Suggested</th></tr></thead>
              <tbody>
                {view.rows.map((r) => (
                  <tr key={r.return_request_id}>
                    <td className="mono">{r.order_id}</td>
                    <td>{r.item}</td>
                    <td className="n">{usd(r.amount)}</td>
                    <td>{r.reason}<small className="mute" style={{ display: "block" }}>{r.evidence}</small></td>
                    <td><span className={`tag ${SUGGESTED[r.suggested][1]}`}>{SUGGESTED[r.suggested][0]}</span><small className="mute" style={{ display: "block" }}>{r.rule}</small></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
        <div>
          <h2>Checks<span className={`r ${toLook ? "warn" : "ok"}`}>{toLook ? `${toLook} to look at` : `${view.checks_total} of ${view.checks_total} passed`}</span></h2>
          <div className="checks">
            {view.checks.map((c) => (
              <div key={c.rule}><span className={`st ${c.passed ? "done" : "need"}`} /><div>{c.title}<small>{c.detail}</small></div></div>
            ))}
          </div>
        </div>
        <div>
          <h2>Messages to read before they go out</h2>
          <div className="grp">
            <dl className="kv">
              {view.messages.map((m) => [<dt key={m.order_id} className="mono">{m.order_id} · {m.kind.replace(/_/g, " ")}</dt>, <dd key={m.order_id + "t"}>{m.text}</dd>])}
            </dl>
          </div>
        </div>
        {view.verification ? (
          <div className="grp" data-testid="verification">
            <div className="gh"><span className="op">read back</span><b>{view.verification.all_match ? "Every order matches" : "Something does not match"}</b>
              <span className="r">Refunds are final and cannot be undone</span></div>
            <dl className="kv">
              {view.verification.changes.map((c, i) => [<dt key={i}><span className={`st ${c.matches ? "done" : "need"}`} style={{ width: 11, height: 11, marginRight: 6 }} />{c.tool}</dt>, <dd key={i + "d"}>{c.detail}</dd>])}
            </dl>
          </div>
        ) : (
          <div className="after">
            <ol><li>After approval</li><li>{view.after_approval.text}</li></ol>
            {view.status === "waiting_for_approval" && (
              <button className="btn dark" disabled={busy} onClick={onApprove} style={{ marginLeft: "auto" }}>{view.approve_label}</button>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
