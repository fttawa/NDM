import { Inbox, Layers, MoreHorizontal, Pause, Play, Plus, RotateCcw, Sparkles, Trash2, X } from "lucide-react";
import { useEffect } from "react";
import { useFlip } from "./Animated";
import { api, type Task } from "../lib/api";
import { arrangeTasks, taskGroup } from "../lib/task-list";
import { TaskViewOptions } from "./TaskViewOptions";
import { bytes } from "../lib/format";
import { DOWNLOAD_FILTERS, filterTasks, useStore } from "../store";
import { TaskCard } from "./TaskCard";

/** 批量操作栏：有选中任务时替换工具栏右侧按钮 */
function BulkBar({ visible }: { visible: Task[] }) {
  const checked = useStore((s) => s.checked);
  const tasks = useStore((s) => s.tasks);
  const setChecked = useStore((s) => s.setChecked);
  const askRemove = useStore((s) => s.askRemove);
  const chosen = tasks.filter((t) => checked.includes(t.id));
  const size = chosen.reduce((a, t) => a + (t.total ?? 0), 0);
  const allVisible = visible.length > 0 && visible.every((t) => checked.includes(t.id));
  const canStart = chosen.some((t) => t.status === "paused" || t.status === "error");
  const canPause = chosen.some((t) => t.status === "active" || t.status === "waiting");
  const canRedo = chosen.some((t) => t.status !== "active" && t.status !== "waiting");

  return (
    <div className="toolbar-actions bulk-bar">
      <span className="bulk-count">
        已选 <b>{chosen.length}</b> 项{size > 0 ? ` · ${bytes(size)}` : ""}
      </span>
      <button className="btn glass" onClick={() => setChecked(allVisible ? [] : visible.map((t) => t.id))}>
        {allVisible ? "取消全选" : "全选"}
      </button>
      <button className="btn glass" disabled={!canStart} onClick={() => api.resume(checked)}>
        <Play size={15} /> 开始
      </button>
      <button className="btn glass" disabled={!canPause} onClick={() => api.pause(checked)}>
        <Pause size={15} /> 暂停
      </button>
      <button
        className="btn glass"
        disabled={!canRedo}
        onClick={() => chosen.filter((t) => t.status !== "active" && t.status !== "waiting").forEach((t) => api.redownload(t.id))}
      >
        <RotateCcw size={15} /> 重新下载
      </button>
      <button className="btn glass danger" onClick={() => askRemove(checked)}>
        <Trash2 size={15} /> 删除
      </button>
      <button className="icon-btn ghost" title="取消选择 (Esc)" onClick={() => setChecked([])}>
        <X size={17} />
      </button>
    </div>
  );
}

export function TasksView() {
  const tasks = useStore((s) => s.tasks);
  const filter = useStore((s) => s.filter);
  const openAdd = useStore((s) => s.openAdd);
  const options = useStore(s => s.settings?.taskView);
  const list = arrangeTasks(filterTasks(tasks, filter), options);
  const groups = new Map<string, Task[]>();
  for (const task of list) { const key = taskGroup(task, options?.group ?? "none"); const group = groups.get(key) ?? []; group.push(task); groups.set(key, group); }
  const flipRef = useFlip(`${filter}|${list.map((t) => t.id).join(",")}`);
  const checkedCount = useStore((s) => s.checked.length);

  // 切换筛选后，只保留当前列表中可见的选中项
  useEffect(() => {
    const { checked, setChecked, tasks: all } = useStore.getState();
    const visible = new Set(filterTasks(all, filter).map((t) => t.id));
    const kept = checked.filter((id) => visible.has(id));
    if (kept.length !== checked.length) setChecked(kept);
  }, [filter]);
  const setFilter = useStore((s) => s.setFilter);
  const completedCount = tasks.filter((task) => task.status === "completed").length;

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>下载 <span className="page-count">{tasks.length}</span></h1>
        </div>
        <div className="toolbar-actions"><TaskViewOptions /><button className="btn primary" onClick={() => openAdd()}>
          <Plus size={17} /> 新建任务
        </button></div>
      </header>

      <div className="toolbar">
        <div className="download-filters" role="group" aria-label="下载状态">
          {DOWNLOAD_FILTERS.map(({ value, label }) => <button key={value} className={filter === value ? "active" : ""} aria-pressed={filter === value} onClick={() => setFilter(value)}>{value === "all" ? "全部" : value === "error" ? "失败" : label}</button>)}
        </div>
        {checkedCount > 0 ? (
          <BulkBar visible={list} />
        ) : (
        <details className="download-more" key={filter}>
          <summary aria-label="更多下载操作"><MoreHorizontal size={20} /></summary>
          <div className="download-menu glass-strong" onClick={(event) => event.currentTarget.parentElement?.removeAttribute("open")}>
          <button className="btn glass" onClick={() => api.resumeAll()}>
            <Play size={15} /> 全部开始
          </button>
          <button className="btn glass" onClick={() => api.pauseAll()}>
            <Pause size={15} /> 全部暂停
          </button>
          <button className="btn glass" onClick={() => api.clearFinished()} disabled={!completedCount}>
            <Sparkles size={15} /> 清除已完成
          </button>
          </div>
        </details>
        )}
      </div>

      {list.length === 0 ? (
        <div className="empty glass">
          {filter === "all" ? <Layers size={40} strokeWidth={1.4} /> : <Inbox size={40} strokeWidth={1.4} />}
          <div className="empty-title">{filter === "all" ? "还没有下载任务" : "这里空空如也"}</div>
          <div className="muted">点击「新建任务」，或直接按 Ctrl+V 粘贴链接</div>
        </div>
      ) : (
        <div className="task-groups">{Array.from(groups, ([group, items]) => <section key={group}>
          {group && <h2 className="task-group-title">{group}<span className="muted small">{items.length}</span></h2>}
          <div className={options?.layout === "list" ? "task-list" : "task-grid"}>
          {items.map((t) => (
            <div key={t.id} ref={flipRef(t.id)} className="flip-item">
              <TaskCard task={t} compact={options?.layout === "list"} />
            </div>
          ))}
          </div></section>)}</div>
      )}
    </div>
  );
}
