import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    // Dev: proxy API and WebSocket to the Rust server on port 9090.
    // In production the same server serves both the frontend and the API.
    proxy: {
      "/api": "http://localhost:9090",
      "/theme": "http://localhost:9090",
      "/ws": {
        target: "ws://localhost:9090",
        ws: true,
      },
    },
  },
});
