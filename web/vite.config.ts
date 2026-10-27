import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The API runs on 127.0.0.1:8000 (make serve). The web app calls it under /api.
// 127.0.0.1, not localhost: Node may resolve localhost to ::1 while uvicorn listens on IPv4.
const api = process.env.API_URL ?? "http://127.0.0.1:8000";
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: { "/api": { target: api, rewrite: (p) => p.replace(/^\/api/, "") } },
  },
});
