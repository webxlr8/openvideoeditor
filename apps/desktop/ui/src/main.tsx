import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { reportError } from "./notify";
import "./styles.css";

// Last line of defence for async work that slipped past `fire` / `runCommand`: say so instead of failing silently.
window.addEventListener("unhandledrejection", (e) => {
  e.preventDefault();
  reportError(e.reason);
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
