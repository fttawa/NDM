import { ArrowDownToLine, Minus, Plus, Settings2, Square, X } from "lucide-react";
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
  const openAdd = useStore((s) => s.openAdd);
  const activeCount = useStore((s) => s.tasks.filter((t) => t.status === "active").length);

  return (
    <nav className="sidebar" data-tauri-drag-region>
      <div className="logo" data-tauri-drag-region>
        <img src="/app-icon.svg" alt="NDM" />
      </div>
      <div className="nav">
        <button className={`nav-btn ${view === "tasks" ? "active" : ""}`} onClick={() => setView("tasks")} title="任务">
          <ArrowDownToLine size={20} />
          {activeCount > 0 && <span className="badge">{activeCount}</span>}
        </button>
        <button className="nav-btn add" onClick={() => openAdd()} title="新建任务 (Ctrl+N)">
          <Plus size={22} />
        </button>
        <button className={`nav-btn ${view === "settings" ? "active" : ""}`} onClick={() => setView("settings")} title="设置">
          <Settings2 size={20} />
        </button>
      </div>
    </nav>
  );
}

export function Toast() {
  const toast = useStore((s) => s.toast);
  return <div className={`toast glass-strong ${toast ? "show" : ""}`}>{toast}</div>;
}
