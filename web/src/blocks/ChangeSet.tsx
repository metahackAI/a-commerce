import type { ChangeSetView, Group } from "../api";
import { usd } from "../api";

const STATUS: Record<ChangeSetView["status"], [string, string]> = {
  waiting_for_approval: ["Waiting for your approval", "warn"],
  running: ["Running", ""],
  verifying: ["Reading back", ""],
  live: ["Live · verified", "ok"],
  failed: ["Read-back failed", "warn"],
  undone: ["Undone", ""],
};

const TONES = ["t-sand", "t-slate", "t-clay", "t-navy", "t-olive", "t-stone"];
const tone = (name: string) => TONES[[...name].reduce((a, c) => a + c.charCodeAt(0), 0) % TONES.length];

const when = (iso: string | null) =>
  iso ? new Date(iso).toLocaleString("en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false }) : "";

type Props = {
  view: ChangeSetView;
  busy: boolean;
  onApprove: () => void;
  onUndo: () => void;
};

export function ChangeSet({ view, busy, onApprove, onUndo }: Props) {
  const [label, cls] = STATUS[view.status];
  const groups = Object.fromEntries(view.groups.map((g) => [g.op, g])) as Partial<Record<Group["op"], Group>>;
  const failing = view.checks_passed < view.checks_total;
  return (
    <section className="cs" data-testid="change-set">
      <div className="csh">
        <span className="id">#{view.id} · v{view.version}</span>
        <h1>{view.title}</h1>
        <span className={`tag ${cls}`}>{label}</span>
        <div className="sp">
          {view.status === "waiting_for_approval" && (
            <button className="btn dark" disabled={busy || failing} onClick={onApprove}
                    title={failing ? "A check is failing. Ask the agent to fix it first." : undefined}>
              {busy ? "Running…" : "Approve and run"}
            </button>
          )}
          {(view.status === "live" || view.status === "failed") && !view.after_approval.final?.length && (
            <button className="btn" disabled={busy} onClick={onUndo}>Undo all together</button>
          )}
        </div>
      </div>
      <div className="body">
        <div>
          <h2>Your request, as I understood it</h2>
          <div className="quote">
            {view.request.map((r, i) => (
              <span key={i}>{i > 0 && " Then: "}“{r}”</span>
            ))}
          </div>
          {view.understood.length > 0 && <div className="intent">
            {view.understood.map((u) => (
              <div key={u.label}><small>{u.label}</small><b>{u.value}</b><span>{u.detail}</span></div>
            ))}
          </div>}
        </div>

        <div>
          <h2>Checks<span className={`r ${failing ? "warn" : "ok"}`}>{view.checks_passed} of {view.checks_total} passed</span></h2>
          <div className="checks">
            {view.checks.map((c) => (
              <div key={c.rule}><span className={`st ${c.passed ? "done" : "need"}`} /><div>{c.title}<small>{c.detail}</small></div></div>
            ))}
          </div>
        </div>

        {groups.update && <PriceGroup g={groups.update} />}

        <div className="two">
          <div className="grp">
            {groups.create && (
              <>
                <div className="gh"><span className="op">create</span><b>{groups.create.title}</b><span className="r">{groups.create.summary}</span></div>
                <dl className="kv">
                  {Object.entries(groups.create.fields ?? {}).map(([k, v]) => [<dt key={k}>{k}</dt>, <dd key={k + "v"}>{v}</dd>])}
                </dl>
              </>
            )}
            {groups.exclude && (
              <>
                <div className="gh" style={{ borderTop: "1px solid var(--line2)" }}><span className="op">exclude</span><b>{groups.exclude.title}</b><span className="r">{groups.exclude.summary}</span></div>
                <dl className="kv">
                  {(groups.exclude.rows ?? []).map((r) => [<dt key={r.name}>{r.name}</dt>, <dd key={r.name + "v"}>{r.note}</dd>])}
                </dl>
              </>
            )}
          </div>
          {groups.schedule?.preview && (
            <div className="grp">
              <div className="gh"><span className="op">schedule</span><b>{groups.schedule.title}</b><span className="r">{groups.schedule.summary}</span></div>
              <div className="prev">
                <div className="seg">{groups.schedule.preview.audiences.map((a, i) => <span key={a} className={i === 0 ? "on" : ""}>{a}</span>)}</div>
                <div className="site">
                  <div className="top"><span>northwind.example</span><span>{groups.schedule.preview.date}</span></div>
                  <div className="hero">
                    <div><small>{groups.schedule.preview.eyebrow}</small><b>{groups.schedule.preview.headline}</b><i>{groups.schedule.preview.cta}</i></div>
                    <span className="art t-sand" />
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>

        {view.groups.filter((g) => (g.op === "send" || (g.op === "schedule" && !g.preview))).map((g) => (
          <div className="grp" key={g.title}>
            <div className="gh"><span className="op">{g.op}</span><b>{g.title}</b><span className="r">{g.summary}</span></div>
            <dl className="kv">{Object.entries(g.fields ?? {}).map(([k, v]) => [<dt key={k}>{k}</dt>, <dd key={k + "v"}>{v}</dd>])}</dl>
          </div>
        ))}

        {view.verification ? (
          <div className="grp" data-testid="verification">
            <div className="gh"><span className="op">read back</span><b>{view.verification.all_match ? "Everything matches this change set" : "Something does not match"}</b>
              <span className="r">{view.status === "undone" ? "Undone"
                : view.after_approval.final?.length ? "Cannot be undone"
                : view.after_approval.undo_until ? `Undo all together until ${when(view.after_approval.undo_until)}` : "Undo all together"}</span></div>
            <dl className="kv">
              {view.verification.changes.map((c) => [
                <dt key={c.tool}><span className={`st ${c.matches ? "done" : "need"}`} style={{ width: 11, height: 11, marginRight: 6 }} />{c.tool}</dt>,
                <dd key={c.tool + "v"}>{c.detail}</dd>,
              ])}
            </dl>
          </div>
        ) : (
          <div className="after">
            <ol>
              <li>After approval</li>
              <li>run {view.after_approval.tools.length} tools</li>
              <li>read everything back</li>
              <li>compare with this change set</li>
            </ol>
            {view.after_approval.final?.length ? (
              <div className="undo warn">Cannot be undone<br /><span className="mute">{view.after_approval.final.join("; ")}</span></div>
            ) : view.after_approval.undo_until ? (
              <div className="undo">Undo all together<br /><span className="mute">until {when(view.after_approval.undo_until)}</span></div>
            ) : (
              <div className="undo">Undo all together</div>
            )}
          </div>
        )}
      </div>
    </section>
  );
}

function PriceGroup({ g }: { g: Group }) {
  const rows = g.rows ?? [];
  const shown = rows.slice(0, 4);
  return (
    <div>
      <h2>Changes</h2>
      <div className="grp">
        <div className="gh"><span className="op">{g.op}</span><b>{g.title}</b>{g.summary}<span className="r">{g.undo}</span></div>
        <table>
          <thead><tr><th>Product</th><th className="n">Stock</th><th className="n">Discount</th><th className="n">VIP price</th></tr></thead>
          <tbody>
            {shown.map((r) => (
              <tr key={r.product_id}>
                <td><div className="p"><span className={`art ${tone(r.name)}`} />{r.name}</div></td>
                <td className="n">{r.stock}</td>
                <td className="n">{r.note ? <span className="tag">{r.discount} · {r.note}</span> : r.discount}</td>
                <td className="n"><span className="was">{usd(r.was ?? 0)}</span>{usd(r.now ?? 0)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length > shown.length && <div className="more">Show {rows.length - shown.length} more</div>}
      </div>
    </div>
  );
}
