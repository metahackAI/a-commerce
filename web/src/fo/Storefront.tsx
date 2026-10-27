import { useEffect, useRef, useState } from "react";
import type { AgentStep, CartData, Chip, FoBlock, Profile, Shopper } from "../api";
import { api, shop } from "../api";
import { Cart, Chips, CompareTable, Gallery } from "./blocks";

type Msg =
  | { role: "user"; text: string }
  | { role: "agent"; text: string; model: string; memories: { memory_id: string; text: string; undone?: boolean }[] }
  | { role: "system"; text: string };

type Main = Extract<FoBlock, { ui: "compare_table" | "look_gallery" | "product_list" }>;

const WHO: Record<Shopper, string> = { jamie: "Jamie · VIP", mina: "Mina · first visit", alex: "Alex" };
const SUGGESTIONS = [
  "I need a light jacket for spring trips. Not too warm, and it has to fit in a carry-on.",
  "Which one packs smallest? I hate bulky bags.",
  "The windbreaker. Same size as last time.",
  "Less text. Just show me bigger photos.",
];

export function Storefront({ shopper }: { shopper: Shopper }) {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [chips, setChips] = useState<Chip[]>([]);
  const [main, setMain] = useState<Main | null>(null);
  const [cart, setCart] = useState<CartData | null>(null);
  const [placed, setPlaced] = useState<string | null>(null);
  const [profile, setProfile] = useState<Profile | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [clock, setClock] = useState("");
  const end = useRef<HTMLDivElement>(null);

  const refreshProfile = () => shop.profile(shopper).then(setProfile).catch(() => setProfile(null));
  useEffect(() => {
    refreshProfile();
    api.health().then((h) => setClock(h.clock)).catch(() => setMsgs([{ role: "system", text: "The API is not running. Start it with make serve." }]));
  }, [shopper]);
  useEffect(() => {
    end.current?.scrollIntoView({ behavior: "smooth" }); // never return the Promise some browsers give back
  }, [msgs]);

  async function send(text: string) {
    if (!text.trim() || busy) return;
    setDraft("");
    setBusy(true);
    setMsgs((m) => [...m, { role: "user", text }]);
    try {
      const { events } = await shop.chat(shopper, text);
      const step = events.find((e): e is AgentStep => e.type === "agent_step");
      const blocks = events.filter((e): e is FoBlock => e.type === "ui");
      const memories = blocks.flatMap((b) => (b.ui === "memory_note" ? [b.data] : []));
      for (const b of blocks) {
        if (b.ui === "understood_chips") setChips(b.data.chips);
        if (b.ui === "compare_table" || b.ui === "look_gallery" || b.ui === "product_list") setMain(b);
        if (b.ui === "cart") { setCart(b.data); setPlaced(null); }
      }
      setMsgs((m) => [...m, { role: "agent", text: step?.text ?? "", model: step?.model ?? "", memories }]);
      if (memories.length) refreshProfile();
    } catch (e) {
      setMsgs((m) => [...m, { role: "system", text: e instanceof Error ? e.message : String(e) }]);
    } finally {
      setBusy(false);
    }
  }

  async function undoMemory(id: string) {
    await shop.undoMemory(shopper, id);
    setMsgs((m) => m.map((x) => (x.role === "agent" ? { ...x, memories: x.memories.map((mm) => (mm.memory_id === id ? { ...mm, undone: true } : mm)) } : x)));
    refreshProfile();
  }

  async function placeOrder() {
    if (!cart) return;
    setBusy(true);
    try {
      const o = await shop.placeOrder(shopper, cart.order_preview_id);
      setPlaced(o.order_id);
    } catch (e) {
      setMsgs((m) => [...m, { role: "system", text: e instanceof Error ? e.message : String(e) }]);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="fo">
      <header className="sh">
        <span className="logo">NORTHWIND</span>
        <nav><span>New in</span><span>Men</span><span>Women</span><span className="vip">VIP weekend</span><span>Orders</span></nav>
        <div className="r">
          <span className="mute">{clock.replace("T", " ")}</span>
          {(Object.keys(WHO) as Shopper[]).map((s) => (
            <a key={s} className={`pill ${s === shopper ? "on" : ""}`} href={`/fo?shopper=${s}`}>{WHO[s]}</a>
          ))}
          <a className="pill" href="/">Back office</a>
          <span className="cart">Bag {cart && !placed ? cart.lines.length : 0}</span>
        </div>
      </header>
      <div className="wrap">
        <section className="conv">
          <div className="msgs">
            {msgs.length === 0 && (
              <div className="empty">
                <p className="mute">Ask in your own words. The page changes to fit what you ask.</p>
                {SUGGESTIONS.slice(0, 1).map((s) => <button key={s} className="chip" disabled={busy} onClick={() => send(s)}>{s}</button>)}
              </div>
            )}
            {msgs.map((m, i) =>
              m.role === "user" ? <div key={i} className="u">{m.text}</div>
              : m.role === "agent" ? (
                <div key={i} className="a">
                  <div className="who"><span className="mark">N</span>Northwind <span className="mute">· {m.model}</span></div>
                  {m.text}
                  {m.memories.map((mm) => (
                    <div key={mm.memory_id} className="mem" data-testid="memory">
                      {mm.undone ? <span className="mute">Forgotten</span> : <>{mm.text}<button className="undo" onClick={() => undoMemory(mm.memory_id)}>Undo</button></>}
                    </div>
                  ))}
                </div>
              ) : <div key={i} className="sys mute">{m.text}</div>,
            )}
            {busy && <div className="sys mute">…</div>}
            <div ref={end} />
          </div>
          <div className="cfoot">
            <form className="composer" onSubmit={(e) => { e.preventDefault(); send(draft); }}>
              <textarea rows={2} value={draft} placeholder="Ask about fit, fabric, delivery, returns…"
                        onChange={(e) => setDraft(e.target.value)}
                        onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(draft); } }} />
              <div className="cbar"><button className="send" type="submit" disabled={busy} aria-label="Send">↑</button></div>
            </form>
          </div>
        </section>

        <section className="canvas">
          {chips.length > 0 && <Chips chips={chips} onRemove={(l) => setChips((c) => c.filter((x) => x.label !== l))} />}
          {main?.ui === "compare_table" && <CompareTable data={main.data} why={main.why} />}
          {(main?.ui === "look_gallery" || main?.ui === "product_list") && <Gallery data={main.data} why={main.why} big={main.ui === "look_gallery"} />}
          {!main && <p className="mute">What you ask for appears here, laid out for you.</p>}
        </section>

        <aside className="cartp">
          {cart ? <Cart data={cart} placed={placed} busy={busy} onPlace={placeOrder} /> : <h3>Your cart is empty</h3>}
          <div data-testid="profile">
            <div className="lbl">What Northwind knows about you</div>
            {profile?.preferences.length ? (
              <div className="ua">{profile.preferences.map((p) => <span key={p.id} className="chip">{p.key === "size" ? `Size ${p.value}` : p.value}</span>)}</div>
            ) : <p className="note">Nothing yet.</p>}
          </div>
        </aside>
      </div>
    </div>
  );
}
