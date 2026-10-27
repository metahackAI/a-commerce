import React from "react";
import ReactDOM from "react-dom/client";
import type { Shopper } from "./api";
import "./base.css";
import "./bo/changeset.css";
import "./bo/extra.css";
import "./fo/store.css";
import "./fo/extra.css";
import { BackOffice } from "./bo/BackOffice";
import { Storefront } from "./fo/Storefront";

function App() {
  // / is the back office (layout A); /fo?shopper=jamie|mina|alex is the storefront.
  if (location.pathname.startsWith("/fo")) {
    const s = new URLSearchParams(location.search).get("shopper");
    const shopper: Shopper = s === "mina" || s === "alex" ? s : "jamie";
    return <Storefront key={shopper} shopper={shopper} />;
  }
  return <BackOffice />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
