import { useEffect, useRef, useState } from "react";
import type { AgentStep, AnyView, Event } from "../api";
import { api, ApiError } from "../api";
import { ChangeSet } from "../blocks/ChangeSet";
import { RefundReview } from "../blocks/RefundReview";

type Msg =
  | { role: "user"; text: string; at: string }
  | { role: "agent"; step: AgentStep; ref?: { id: number; version: number } }
  | { role: "system"; text: string };

const SUGGESTION = "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.";
const SUGGESTION_RETURNS = "Check returns waiting over $200";
const CLOCKS: [string, string][] = [["Fri Nov 6, 11:02", "2026-11-06T11:02"], ["Sat Nov 7, 10:00", "2026-11-07T10:00"], ["Tue Nov 10", "2026-11-10T09:00"]];

const hhmm = (iso: string) => iso.slice(11, 16);

export function BackOffice() {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [view, setView] = useState<AnyView | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [clock, setClock] = useState("");
  const [model, setModel] = useState("");
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.health().then((h) => setClock(h.clock)).catch(() => setMsgs([{ role: "system", text: "The API is not running. Start it with make serve." }]));
  }, []);
  useEffect(() => {
    end.current?.scrollIntoView({ behavior: "smooth" }); // newer browsers return a Promise here; never return it
  }, [msgs]);

  async function send(text: string) {
    if (!text.trim() || busy) return;
    setDraft("");
    setBusy(true);
    setMsgs((m) => [...m, { role: "user", text, at: clock }]);
    try {
      // A message about an open sale change set edits it; anything else starts a new task.
      const editing = view && view.status === "waiting_for_approval" && view.kind !== "refund_review" && !/return|refund/i.test(text);
      const res = await api.operatorChat(text, editing ? view.id : null);
      const step = res.events.find((e): e is AgentStep => e.type === "agent_step");
      const block = res.events.find((e: Event) => e.type === "ui");
      const handoff = res.events.find((e: Event) => e.type === "handoff");
      if (handoff && handoff.type === "handoff") setMsgs((m) => [...m, { role: "system", text: `${handoff.from} handed this to ${handoff.to}.` }]);
      if (step) {
        setModel(step.model);
        setMsgs((m) => [...m, { role: "agent", step, ref: block && block.type === "ui" ? { id: block.data.id, version: block.data.version } : undefined }]);
      }
      if (block && block.type === "ui") setView(block.data);
    } catch (e) {
      setMsgs((m) => [...m, { role: "system", text: e instanceof Error ? e.message : String(e) }]);
    } finally {
      setBusy(false);
    }
  }

  async function act(fn: () => Promise<AnyView>, note: (v: AnyView) => string) {
    setBusy(true);
    try {
      const v = await fn();
      setView(v);
      setMsgs((m) => [...m, { role: "system", text: note(v) }]);
    } catch (e) {
      setMsgs((m) => [...m, { role: "system", text: e instanceof ApiError ? e.message : String(e) }]);
    } finally {
      setBusy(false);
    }
  }

  async function changeClock(at: string) {
    setBusy(true);
    try {
      await api.setClock(at);
      setClock(at);
    } finally {
      setBusy(false);
    }
  }

  async function reset() {
    setMsgs([]); // clear first, so a message sent right after a reset is not wiped when the reset returns
    setView(null);
    setBusy(true);
    try {
      const h = await api.reset();
      setClock(h.clock);
    } finally {
      setBusy(false);
    }
  }

  const editing = view?.status === "waiting_for_approval";
  return (
    <div className="bo"><div className="app">
      <aside className="side">
        <div className="storesw"><span className="mark">N</span><div><b>Northwind Apparel</b><small>Back office</small></div></div>
        <button className="nav">Inbox<span className="cnt">3</span></button>
        <button className="nav on">Change sets</button>
        <button className="nav">Automations<span className="cnt">4</span></button>
        <button className="nav">Catalog</button>
        <button className="nav">Customers</button>
        <button className="nav">Policies</button>
        <button className="nav">Activity</button>
        {view && (
          <>
            <div className="sect">Open change sets</div>
            <button className="nav on" style={{ fontWeight: 400 }}>
              <span className={`st ${view.status === "live" ? "done" : view.status === "undone" ? "" : "need"}`} style={{ width: 12, height: 12 }} />
              {view.title.split(":")[0]}
            </button>
          </>
        )}
        <div className="demo" data-testid="demo-clock">
          <div className="sect" style={{ padding: "0 0 4px" }}>Demo clock · {clock.replace("T", " ")}</div>
          {CLOCKS.map(([label, at]) => (
            <button key={at} className={`pill ${clock === at ? "on" : ""}`} disabled={busy} onClick={() => changeClock(at)}>{label}</button>
          ))}
          <button className="pill" disabled={busy} onClick={reset}>Reset data</button>
          <a className="pill" href="/fo?shopper=jamie">Open storefront →</a>
        </div>
        <div className="me"><span className="av">SJ</span><div>Son SeongJun<small>Merchandiser</small></div></div>
      </aside>

      <section className="conv">
        <div className="ch">{view ? view.title.split(":")[0] : "New request"}</div>
        <div className="msgs">
          {msgs.length === 0 && (
            <div className="empty">
              <p className="mute">Tell the agent what the store should do. Nothing changes until you approve.</p>
              <button className="chip" disabled={busy} onClick={() => send(SUGGESTION)}>{SUGGESTION}</button>
              <button className="chip" disabled={busy} onClick={() => send(SUGGESTION_RETURNS)}>{SUGGESTION_RETURNS}</button>
            </div>
          )}
          {msgs.map((m, i) =>
            m.role === "user" ? (
              <div key={i} style={{ display: "contents" }}>
                <div className="u">{m.text}</div>
                <div className="ts">{hhmm(m.at)}</div>
              </div>
            ) : m.role === "agent" ? (
              <div key={i} className="a">
                <div className="who"><span className="mark">A</span>Operator agent</div>
                {m.step.text}
                {m.step.corrected && (
                  <details className="corrected" data-testid="corrected">
                    <summary>Reply checked against the change set: {m.step.corrected.issues.length} correction{m.step.corrected.issues.length > 1 ? "s" : ""}</summary>
                    <ul>{m.step.corrected.issues.map((x) => <li key={x}>{x}</li>)}</ul>
                    <p className="mute">The model said: “{m.step.corrected.said}”</p>
                  </details>
                )}
                <div className="calls"><span>{m.step.tool_calls} tool calls · {m.step.model}</span><span>{(m.step.duration_ms / 1000).toFixed(1)}s</span></div>
                {m.ref && <div><span className="ref"><span className="st need" style={{ width: 11, height: 11 }} />Change set #{m.ref.id} · version {m.ref.version}</span></div>}
              </div>
            ) : (
              <div key={i} className="sys mute">{m.text}</div>
            ),
          )}
          {busy && <div className="sys mute">Working…</div>}
          <div ref={end} />
        </div>
        <div className="cfoot">
          <form className="composer" onSubmit={(e) => { e.preventDefault(); send(draft); }}>
            <textarea className="ph" rows={2} value={draft} placeholder={editing ? "Ask for changes to this set…" : "Describe a task for the store…"}
                      onChange={(e) => setDraft(e.target.value)}
                      onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(draft); } }} />
            <div className="cbar">
              <span className="pill">{model || "model"} · auto</span>
              <button className="send" type="submit" disabled={busy} aria-label="Send">↑</button>
            </div>
          </form>
        </div>
      </section>

      {view?.kind === "refund_review" ? (
        <RefundReview view={view} busy={busy}
          onApprove={() => act(() => api.approve(view.id, view.version), (v) => v.status === "live"
            ? `Approved. ${v.kind === "refund_review" ? v.after_approval.text : ""}: every order read back and matches.`
            : "Approved, but the read-back found a mismatch.")} />
      ) : view ? (
        <ChangeSet view={view} busy={busy}
          onApprove={() => act(() => api.approve(view.id, view.version), (v) => v.status === "live"
            ? `Approved v${v.version}. Ran ${v.after_approval.tools.length} tools and read everything back: all match.`
            : `Approved v${v.version}, but the read-back found a mismatch.`)}
          onUndo={() => act(() => api.undo(view.id), (v) => `Undid change set #${v.id}. Prices, coupon and banner are back to before.`)} />
      ) : (
        <section className="cs"><div className="csh"><h1 className="mute">Change set</h1></div>
          <div className="body"><p className="mute">The change set appears here: what the agent understood, the checks, and every change grouped for review.</p></div></section>
      )}
    </div></div>
  );
}
