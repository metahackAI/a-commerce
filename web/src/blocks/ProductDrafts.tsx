import { useState } from "react";
import type { DraftsView } from "../api";
import { usd } from "../api";

export function ProductDrafts({ view, busy, onApprove, onUndo }: { view: DraftsView; busy: boolean; onApprove: () => void; onUndo: () => void }) {
  const [all, setAll] = useState(false);
  const shown = all ? view.items : view.items.slice(0, 8);
  const status = view.status === "live" ? ["Saved · verified", "ok"] : view.status === "undone" ? ["Undone", ""] : ["Ready to save", "warn"];
  return (
    <section className="cs" data-testid="product-drafts">
      <div className="csh"><span className="id">{view.ready} of {view.rows}</span><h1>{view.title}</h1><span className={`tag ${status[1]}`}>{status[0]}</span></div>
      <div className="body">
        <div className="intent" style={{ gridTemplateColumns: "repeat(3,1fr)" }}>
          <div><small>From the sheet</small><b>{view.rows} rows</b><span>{view.sheet}</span></div>
          <div><small>Ready</small><b>{view.ready}</b><span>all fields filled</span></div>
          <div><small>Needs you</small><b>{view.needs_you}</b><span>missing or unclear</span></div>
        </div>
        <div>
          <h2>Drafts<span className="r">supplier codes turned into names, prices from the pricing rule</span></h2>
          <div className="grp">
            <table>
              <thead><tr><th>Product</th><th className="n">Cost → price</th><th>Sizes</th><th>Status</th></tr></thead>
              <tbody>
                {shown.map((d) => (
                  <tr key={d.row}>
                    <td>{d.name}<small className="mono" style={{ display: "block" }}>{d.code}</small></td>
                    <td className="n"><span className="mute">{usd(d.cost)} →</span> {usd(d.price)}</td>
                    <td>{d.sizes}</td>
                    <td>{d.status === "ready" ? <span className="tag ok">Ready</span> : <span className="tag warn">{d.problem}</span>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!all && view.items.length > shown.length && <button className="more" onClick={() => setAll(true)}>Show {view.items.length - shown.length} more</button>}
          </div>
        </div>
        <div className="two">
          <div className="grp"><div className="gh"><b>Rules used</b></div>
            <dl className="kv">{view.rules.map(([k, v]) => [<dt key={k}>{k}</dt>, <dd key={k + "v"}>{v}</dd>])}</dl></div>
          <div className="grp"><div className="gh"><b>Checks</b><span className="r">{view.checks_passed} of {view.checks_total} passed</span></div>
            <dl className="kv">{view.checks.map((c) => [<dt key={c.rule}><span className={`st ${c.passed ? "done" : "need"}`} style={{ width: 11, height: 11 }} /></dt>,
              <dd key={c.rule + "v"}>{c.title}<small className="mute" style={{ display: "block" }}>{c.detail}</small></dd>])}</dl></div>
        </div>
        {view.verification && (
          <div className="grp" data-testid="verification">
            <div className="gh"><span className="op">read back</span><b>{view.verification.all_match ? "Saved and read back" : "Something does not match"}</b></div>
            <dl className="kv">{view.verification.changes.map((c, i) => [<dt key={i}>{c.tool}</dt>, <dd key={i + "d"}>{c.detail}</dd>])}</dl>
          </div>
        )}
        <div className="after">
          <ol><li>{view.after_approval.text}</li></ol>
          {view.status === "waiting_for_approval" && <button className="btn dark" disabled={busy} onClick={onApprove} style={{ marginLeft: "auto" }}>{view.approve_label}</button>}
          {view.status === "live" && <button className="btn" disabled={busy} onClick={onUndo} style={{ marginLeft: "auto" }}>Remove these drafts</button>}
        </div>
      </div>
    </section>
  );
}
