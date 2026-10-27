import { useState } from "react";
import type { FitData, GiftData } from "../api";
import { usd } from "../api";
import { PriceView, tone } from "./blocks";

export function FitCheck({ data, onAdd }: { data: FitData; onAdd: (size: string) => void }) {
  return (
    <div data-testid="fit_check" style={{ display: "flex", flexDirection: "column", gap: 18 }}>
      <div className="line">
        <div className={`art ${tone(data.product.name)}`} />
        <div><b>{data.product.name}</b><small>{data.product.color} · {data.product.fabric} · {usd(data.product.price)}</small><small>{data.product.exchange}</small></div>
      </div>
      <div className="opts">
        {data.recommendations.map((r) => (
          <div key={r.wearing} className="opt on">
            <small className="mute">{r.wearing}</small>
            <b style={{ fontSize: 22 }}>{r.size}</b>
            <span className="note">{r.why}</span>
            <button className="btn sm" onClick={() => onAdd(r.size)}>Add {r.size} · {usd(data.product.price)}</button>
          </div>
        ))}
      </div>
      <div>
        <h2>Measurements<span>compared with something you own</span></h2>
        <div className="cmp" style={{ marginTop: 8 }}>
          <table><thead><tr><th />{data.columns.map((c) => <th key={c}>{c}</th>)}</tr></thead>
            <tbody>{data.rows.map((r) => <tr key={r.label}><td>{r.label}</td>{r.values.map((v, i) => <td key={i}>{v}</td>)}</tr>)}</tbody></table>
        </div>
      </div>
      {data.reviews && (
        <div className="note">From reviews · {data.reviews.people} people {data.reviews.heights}: {data.reviews.true_to_size} say true to size,
          {" "}{data.reviews.snug} say a bit snug{Object.entries(data.reviews.phrases).map(([k, n]) => ` · “${k}” ${n} times`)}</div>
      )}
      <div className="ua"><span className="l">What I used</span>{data.used.map((u) => <span key={u} className="chip">{u}</span>)}
        {data.not_used.map((u) => <span key={u} className="chip mute">Not used: {u}</span>)}</div>
    </div>
  );
}

export function GiftPicker({ data, busy, placed, onPlace }: {
  data: GiftData; busy: boolean; placed: string | null;
  onPlace: (o: { product_id: string; size: string; wrap: boolean; message: string; recipient: string; address: string }) => void;
}) {
  const [chosen, setChosen] = useState(data.chosen);
  const [wrap, setWrap] = useState(true);
  const [message, setMessage] = useState("");
  const [recipient, setRecipient] = useState("");
  const [address, setAddress] = useState("");
  const pick = data.picks.find((p) => p.product_id === chosen) ?? data.picks[0];
  const total = pick.price.now + (wrap ? 5 : 0);
  return (
    <div data-testid="gift_picker" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <h2>Three picks<span>nothing here is saved to your profile</span></h2>
      <div className="opts" style={{ gridTemplateColumns: "repeat(3,1fr)" }}>
        {data.picks.map((p) => (
          <button key={p.product_id} className={`opt ${p.product_id === chosen ? "on" : ""}`} onClick={() => setChosen(p.product_id)} disabled={!!placed}>
            <div className={`art ${tone(p.name)}`} style={{ height: 90 }} />
            <b>{p.name}</b><PriceView p={p.price} />
            <span className="note">{p.product_id === chosen ? `Chosen${data.size ? ` · ${data.size}` : ""}` : "Choose"}</span>
          </button>
        ))}
      </div>
      <div className="grp" style={{ padding: 12, display: "grid", gap: 8 }}>
        <label><input type="checkbox" checked={wrap} onChange={(e) => setWrap(e.target.checked)} /> {data.options.wrap}</label>
        <input placeholder="Card message" value={message} onChange={(e) => setMessage(e.target.value)} />
        <span className="note">{data.options.receipt} · {data.options.exchange}</span>
      </div>
      <div className="grp" style={{ padding: 12, display: "grid", gap: 8 }}>
        <input placeholder="Who it is for" value={recipient} onChange={(e) => setRecipient(e.target.value)} />
        <input placeholder="Their address" value={address} onChange={(e) => setAddress(e.target.value)} />
        <span className="note">Arrives {data.delivery.arrives}{data.delivery.note ? `, ${data.delivery.note.toLowerCase()}` : ""}. {data.address_note}</span>
      </div>
      <div className="sum tot"><span>Total</span><span>{usd(total)}</span></div>
      {placed ? <div className="tag ok" data-testid="gift-placed">Gift order {placed} placed</div> : (
        <button className="btn dark wide" disabled={busy || !recipient || !address || !data.size}
                onClick={() => onPlace({ product_id: pick.product_id, size: data.size ?? "", wrap, message, recipient, address })}>
          {data.size ? `Place gift order · ${usd(total)}` : "Tell me the size first"}
        </button>
      )}
      <p className="note">Northwind prepares the order. Only you can place it.</p>
    </div>
  );
}
