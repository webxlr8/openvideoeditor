/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The UI lives in ./ui; the Tauri crate in ./src-tauri embeds ./ui/dist.
export default defineConfig({
  root: "ui",
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true, target: "es2022" },
  test: {
    root: ".",
    include: ["ui/src/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    setupFiles: ["ui/src/test-setup.ts"],
  },
});
