import React from "react";
import ReactDOM from "react-dom/client";
import "./base.css";
import "./bo/changeset.css";
import "./bo/extra.css";
import { BackOffice } from "./bo/BackOffice";

function App() {
  // /bo is the back office (layout A). The storefront (/fo) comes next.
  return <BackOffice />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
