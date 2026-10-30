import { ArrowDownWideNarrow, ArrowUpNarrowWide, LayoutGrid, List, SlidersHorizontal } from "lucide-react";
import { useId, useLayoutEffect, useRef, useState } from "react";
import { type TaskViewOptions as ViewOptions } from "../lib/api";
import { useStore } from "../store";
import { Switch } from "./ui";

const sorts = [
  { value: "created", label: "添加时间" },
  { value: "name", label: "文件名称" },
  { value: "type", label: "文件类型" },
  { value: "size", label: "文件大小" },
] as const;
const groups = [
  { value: "none", label: "不分组" },
  { value: "date", label: "添加日期" },
  { value: "type", label: "文件类型" },
  { value: "status", label: "下载状态" },
] as const;

export function TaskViewOptions() {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const overlayOpen = useStore(s => s.addOpen || !!s.refreshId || !!s.confirmIds || !!s.selectedId);
  const options = useStore(s => s.settings?.taskView);
  const save = useStore(s => s.saveSettings);
  const change = (patch: Partial<ViewOptions>) => options && save({ taskView: { ...options, ...patch } });
  const descending = options?.descending ?? true;

  useLayoutEffect(() => {
    // 应用弹窗不使用浏览器的 dialog，需要主动让出顶层。
    if (overlayOpen) panel.current?.hidePopover();
  }, [overlayOpen]);

  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      if (!trigger.current || !panel.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const menu = panel.current.getBoundingClientRect();
      const left = Math.max(12, Math.min(anchor.right - menu.width, window.innerWidth - menu.width - 12));
      const top = Math.max(12, Math.min(anchor.bottom + 8, window.innerHeight - menu.height - 12));
      panel.current.style.left = `${left}px`;
      panel.current.style.top = `${top}px`;
    };
    position();
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    return () => {
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
    };
  }, [open]);

  return <>
    <button ref={trigger} type="button" className="icon-btn ghost view-options-trigger"
      aria-label="下载视图选项" title="视图与排序" aria-expanded={open} aria-controls={id}
      aria-haspopup="dialog" popoverTarget={id}>
      <SlidersHorizontal size={18} />
    </button>
    <div ref={panel} id={id} className="view-options-panel" popover="auto" role="dialog"
      aria-label="视图与排序" onToggle={event => setOpen(event.newState === "open")}>
      <div className="view-options-title">视图与排序</div>
      <fieldset className="view-options-section">
        <legend>显示方式</legend>
        <div className="view-options-choices">
          <button type="button" aria-pressed={(options?.layout ?? "cards") === "cards"} onClick={() => change({ layout: "cards" })}><LayoutGrid size={15} />卡片</button>
          <button type="button" aria-pressed={options?.layout === "list"} onClick={() => change({ layout: "list" })}><List size={15} />紧凑列表</button>
        </div>
      </fieldset>
      <div className="view-options-section">
        <div className="view-options-sort-heading">
          <span id={`${id}-sort`}>排序方式</span>
          <button type="button" className="view-options-direction" aria-label="切换排序方向" onClick={() => change({ descending: !descending })}>
            {descending ? <ArrowDownWideNarrow size={14} /> : <ArrowUpNarrowWide size={14} />}{descending ? "降序" : "升序"}
          </button>
        </div>
        <div className="view-options-choices" role="group" aria-labelledby={`${id}-sort`}>
          {sorts.map(({ value, label }) => <button key={value} type="button" aria-pressed={(options?.sort ?? "created") === value} onClick={() => change({ sort: value })}>{label}</button>)}
        </div>
      </div>
      <fieldset className="view-options-section">
        <legend>分组方式</legend>
        <div className="view-options-choices">
          {groups.map(({ value, label }) => <button key={value} type="button" aria-pressed={(options?.group ?? "none") === value} onClick={() => change({ group: value })}>{label}</button>)}
        </div>
      </fieldset>
      <div className="view-options-percentage">
        <span>显示下载百分比</span>
        <Switch label="显示下载百分比" checked={options?.showPercentage ?? true} onChange={showPercentage => change({ showPercentage })} />
      </div>
    </div>
  </>;
}
