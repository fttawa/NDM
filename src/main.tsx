import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { BatchCard } from "./components/BatchCard";
import { CaptureCard } from "./components/CaptureCard";
import "./styles.css";

const captureId = /^#capture\/(.+)$/.exec(location.hash)?.[1];
const batchId = /^#batch\/(.+)$/.exec(location.hash)?.[1];
if (captureId || batchId) document.documentElement.dataset.view = "capture";
document.documentElement.dataset.env = "__TAURI_INTERNALS__" in window ? "app" : "web";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {/* #capture/<id>：下载确认小卡片窗口；其余为主窗口 */}
    {captureId ? <CaptureCard id={captureId} /> : batchId ? <BatchCard id={batchId} /> : <App />}
  </React.StrictMode>,
);
