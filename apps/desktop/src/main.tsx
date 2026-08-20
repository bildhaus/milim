import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { recordFrontendError } from "./api";
import { markPerfStage } from "./lib/perf";
import "./styles.css";

markPerfStage("entry_evaluated");

window.addEventListener("error", (event) => {
  const error = event.error instanceof Error ? event.error : null;
  void recordFrontendError(
    error?.message || event.message || "Unknown window error",
    error?.stack,
  ).catch(() => {});
});

window.addEventListener("unhandledrejection", (event) => {
  const error = event.reason instanceof Error ? event.reason : null;
  void recordFrontendError(
    error?.message || "Unhandled promise rejection",
    error?.stack,
  ).catch(() => {});
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
