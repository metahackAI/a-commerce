import type { CartData, Chip, CompareData, GalleryData, Price } from "../api";
import { usd } from "../api";

const TONES = ["t-sand", "t-slate", "t-clay", "t-navy", "t-olive", "t-stone"];
export const tone = (name: string) => TONES[[...name].reduce((a, c) => a + c.charCodeAt(0), 0) % TONES.length];

const isPrice = (v: unknown): v is Price => typeof v === "object" && v !== null && "now" in v;

export function PriceView({ p }: { p: Price }) {
  return (
    <div className="price">
      <b>{usd(p.now)}</b>
      {p.was != null && <s>{usd(p.was)}</s>}
      {p.note && <small>{p.note}</small>}
    </div>
  );
}

export function Chips({ chips, onRemove }: { chips: Chip[]; onRemove: (label: string) => void }) {
  return (
    <div className="ua" data-testid="chips">
      <span className="l">Looking for</span>
      {chips.map((c) => (
        <span key={c.label} className="chip">
          {c.label}
          {c.source === "profile" && <small>your profile</small>}
          <button className="x" aria-label={`Remove ${c.label}`} onClick={() => onRemove(c.label)}>×</button>
        </span>
      ))}
    </div>
  );
}

export function CompareTable({ data, why }: { data: CompareData; why: string }) {
  return (
    <div data-testid="compare_table">
      <h2>Side by side<span>sorted by {data.sort === "weight" ? "weight" : data.sort}</span></h2>
      <div className="cmp" style={{ marginTop: 10 }}>
        <table>
          <thead>
            <tr>
              <th />
              {data.products.map((p, i) => (
                <th key={p.product_id} className={i === 0 ? "best" : ""}>
                  <div className={`art ${tone(p.name)}`} />
                  {i === 0 && data.sort === "weight" && <span className="bmark">Lightest</span>}
                  <b>{p.name}</b>
                  <small>{p.sub}</small>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {data.rows.map((r) => (
              <tr key={r.label}>
                <td>{r.label}</td>
                {r.values.map((v, i) => (
                  <td key={i} className={i === 0 ? "best" : ""}>{isPrice(v) ? <PriceView p={v} /> : v}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="note" style={{ marginTop: 8 }}>{why}</p>
    </div>
  );
}

export function Gallery({ data, why, big }: { data: GalleryData; why: string; big: boolean }) {
  return (
    <div data-testid={big ? "look_gallery" : "product_list"}>
      <div className={big ? "grid big" : "list"}>
        {data.items.map((it) => (
          <div key={it.product_id} className={big ? "card" : "row"}>
            <div className={`art ${tone(it.name)}`} />
            <div>
              <b>{it.name}</b>
              <PriceView p={it.price} />
              {big ? it.look && <small>{it.look}</small> : <small>{it.facts.join(" · ")}</small>}
            </div>
          </div>
        ))}
      </div>
      <p className="note" style={{ marginTop: 8 }}>{why}</p>
    </div>
  );
}

export function Cart({ data, placed, busy, onPlace }: { data: CartData; placed: string | null; busy: boolean; onPlace: () => void }) {
  return (
    <div data-testid="cart" style={{ display: "flex", flexDirection: "column", gap: 14 }}>
      <h3>Your cart</h3>
      {data.lines.map((l) => (
        <div key={l.product_id + l.variant} className="line">
          <div className={`art ${tone(l.name)}`} />
          <div><b>{l.name}</b><small>{l.variant} · {l.qty}</small></div>
        </div>
      ))}
      <div>
        <div className="sum"><span>Subtotal</span><span>{usd(data.subtotal)}</span></div>
        {data.benefit && <div className="sum"><span>{data.benefit.label}</span><span className="g">−{usd(-data.benefit.amount)}</span></div>}
        <div className="sum"><span>Shipping</span><span>{data.shipping === 0 ? "Free" : usd(data.shipping)}</span></div>
        <div className="sum tot"><span>Total</span><span>{usd(data.total)}</span></div>
      </div>
      <div><div className="lbl">Ship to</div>{data.ship_to}<div className="note">Arrives {data.arrives}</div></div>
      <div><div className="lbl">Pay with</div>{data.pay_with}</div>
      {placed ? (
        <div className="tag ok" data-testid="placed">Order {placed} placed</div>
      ) : (
        <button className="btn dark wide" disabled={busy} onClick={onPlace}>Place order · {usd(data.total)}</button>
      )}
      <p className="note">{data.notice}</p>
    </div>
  );
}
