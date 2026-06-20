import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Standalone web app (NOT a rustyredcore_THG cargo member). The browser adapter
// for the theorem-copresence substrate: Velt + Tiptap CRDT (Yjs) collaborative
// editing, plus a browser-driven Gemma co-writer.
export default defineConfig({
  plugins: [react()],
  server: { port: 5173 },
});
