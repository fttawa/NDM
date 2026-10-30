import { useEffect, useRef, useState } from "react";
import { AddTaskModal, ConfirmRemoveModal, RefreshLinkModal } from "./components/Modals";
import { SettingsView } from "./components/SettingsView";
import { Sidebar, TitleBar, Toast } from "./components/Shell";
import { TaskDrawer } from "./components/TaskDrawer";
import { HomeView } from "./components/HomeView";
import { TasksView } from "./components/TasksView";
import { BtSettingsView, SettingsShell } from "./components/BtSettingsView";
import { BtDetailView } from "./components/BtDetailView";
import { UpdateNotice } from "./components/UpdateNotice";
import { CompletionNotice, ScheduleView } from "./components/ScheduleView";
import { api, isTauri, readClipboard, type Settings } from "./lib/api";
import { extractUrls } from "./lib/format";
import { filterTasks, useStore } from "./store";

const isEditable = (el: EventTarget | null) =>
  el instanceof HTMLElement && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);

export default function App() {
  const init = useStore((s) => s.init);
  const view = useStore((s) => s.view);
  const settingsTab = view === "settings" ? "general" : view === "bt-settings" ? "bt" : view === "schedule" ? "schedule" : null;
  const settings = useStore((s) => s.settings);
  const openAdd = useStore((s) => s.openAdd);
  const lastClip = useRef<string | null>(null);
  // 实际生效的窗口材质：最大化时后端会把亚克力临时换成云母
  const [effectiveGlass, setEffectiveGlass] = useState<Settings["glass"] | null>(null);

  useEffect(() => {
    api.effectiveGlass().then(setEffectiveGlass).catch(() => {});
    const off = api.onGlass(setEffectiveGlass);
    return () => {
      off.then((f) => f());
    };
  }, [settings?.glass]);

  useEffect(() => {
    init();
  }, [init]);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.glass = effectiveGlass ?? settings?.glass ?? "acrylic";
    root.dataset.theme = settings?.theme ?? "default";
    root.dataset.env = isTauri ? "app" : "web";
  }, [settings?.glass, settings?.theme, effectiveGlass]);

  // 剪贴板监听：窗口获得焦点时检测新的下载链接
  useEffect(() => {
    if (!settings?.clipboardWatch) return;
    const check = async () => {
      const text = (await readClipboard()).trim();
      if (lastClip.current === null) {
        lastClip.current = text; // 启动时的剪贴板内容不触发
        return;
      }
      if (!text || text === lastClip.current) return;
      lastClip.current = text;
      const s = useStore.getState();
      if (s.addOpen || s.refreshId) return;
      const urls = extractUrls(text).filter((u) => !s.tasks.some((t) => t.url === u));
      if (urls.length) openAdd(urls.join("\n"));
    };
    check();
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, [settings?.clipboardWatch, openAdd]);

  // 快捷键：Ctrl+N 新建，非输入框内 Ctrl+V 直接粘贴链接
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
        e.preventDefault();
        if (!useStore.getState().refreshId) openAdd();
        return;
      }
      // 任务多选快捷键：仅在任务页、无弹窗/抽屉、焦点不在输入框时生效
      const s = useStore.getState();
      if (s.view !== "tasks" || s.addOpen || s.refreshId || s.confirmIds || s.selectedId || isEditable(e.target)) return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
        e.preventDefault();
        s.setChecked(filterTasks(s.tasks, s.filter).map((t) => t.id));
      } else if (e.key === "Delete" && s.checked.length) {
        s.askRemove(s.checked);
      } else if (e.key === "Escape" && s.checked.length) {
        s.setChecked([]);
      }
    };
    const onPaste = (e: ClipboardEvent) => {
      if (isEditable(e.target) || useStore.getState().addOpen || useStore.getState().refreshId) return;
      const urls = extractUrls(e.clipboardData?.getData("text") ?? "");
      if (urls.length) openAdd(urls.join("\n"));
    };
    window.addEventListener("keydown", onKey);
    document.addEventListener("paste", onPaste);
    return () => {
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("paste", onPaste);
    };
  }, [openAdd]);

  return (
    <div className="app">
      <div className="app-bg" aria-hidden />
      <Sidebar />
      <main className="main">
        <TitleBar />
        {/* 设置的三个分类共用一个外壳，切换分类时标题和标签不重建 */}
        <div className="content" key={settingsTab ? "settings" : view}>
          {view === "home" ? <HomeView /> : view === "tasks" ? <TasksView /> : view === "bt-detail" ? <BtDetailView /> : (
            <SettingsShell active={settingsTab!}>
              {settingsTab === "bt" ? <BtSettingsView /> : settingsTab === "schedule" ? <ScheduleView /> : <SettingsView />}
            </SettingsShell>
          )}
        </div>
      </main>
      <TaskDrawer />
      <UpdateNotice />
      <CompletionNotice />
      <AddTaskModal />
      <ConfirmRemoveModal />
      <RefreshLinkModal />
      <Toast />
    </div>
  );
}
