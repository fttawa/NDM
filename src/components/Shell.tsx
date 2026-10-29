import { ArrowDownToLine, House, Minus, Settings2, Square, X } from "lucide-react";
import { windowAction } from "../lib/api";
import { speed } from "../lib/format";
import { useStore } from "../store";
import { AnimatedNumber } from "./Animated";

export function TitleBar() {
  const total = useStore((s) => s.tasks.reduce((a, t) => a + (t.status === "active" ? t.speed : 0), 0));
  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="tb-speed" data-tauri-drag-region>
        {total > 0 && (
          <>
            <ArrowDownToLine size={13} /> <AnimatedNumber mode="smooth" value={total} format={speed} />
          </>
        )}
      </div>
      <div className="win-ctrls">
        <button onClick={() => windowAction("minimize")} aria-label="最小化">
          <Minus size={15} />
        </button>
        <button onClick={() => windowAction("maximize")} aria-label="最大化">
          <Square size={12} />
        </button>
        <button className="close" onClick={() => windowAction("close")} aria-label="关闭">
          <X size={16} />
        </button>
      </div>
    </div>
  );
}

export function Sidebar() {
  const view = useStore((s) => s.view);
  const setView = useStore((s) => s.setView);

  return (
    <nav className="sidebar" aria-label="主导航">
      <div className="logo" data-tauri-drag-region>
        <img src="/app-icon.svg" alt="" /><strong data-tauri-drag-region>NDM</strong>
      </div>
      <div className="nav">
        <button className={`nav-btn ${view === "home" ? "active" : ""}`} onClick={() => setView("home")} aria-current={view === "home" ? "page" : undefined}>
          <House size={18} /><span>首页</span>
        </button>
        <button className={`nav-btn ${view === "tasks" || view === "bt-detail" ? "active" : ""}`} onClick={() => setView("tasks")} aria-current={view === "tasks" || view === "bt-detail" ? "page" : undefined}>
          <ArrowDownToLine size={18} /><span>下载</span>
        </button>
      </div>
      <div className="nav-footer">
        <button className={`nav-btn ${view === "settings" || view === "bt-settings" ? "active" : ""}`} onClick={() => setView("settings")} aria-current={view === "settings" || view === "bt-settings" ? "page" : undefined}>
          <Settings2 size={18} /><span>设置</span>
        </button>
      </div>
    </nav>
  );
}

export function Toast() {
  const toast = useStore((s) => s.toast);
  return <div className={`toast glass-strong ${toast ? "show" : ""}`}>{toast}</div>;
}
