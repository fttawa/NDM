import type { Task, TaskViewOptions } from "./api";

export const taskType = (task: Task) => task.torrent ? "BT" : task.hls ? "HLS" : (task.fileName.includes(".") ? task.fileName.split(".").pop()!.toUpperCase() : "无扩展名");
const statuses = { active: "下载中", waiting: "等待中", paused: "已暂停", error: "失败", completed: "已完成" };
export function taskGroup(task: Task, group: TaskViewOptions["group"]): string {
  if (group === "type") return taskType(task);
  if (group === "status") return statuses[task.status];
  if (group === "date") {
    const d = new Date(task.createdAt);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }
  return "";
}
export function arrangeTasks(tasks: Task[], options?: TaskViewOptions): Task[] {
  if (!options) return tasks;
  const compare = (a: string, b: string) => a.localeCompare(b, "zh-CN", { numeric: true });
  const direction = options.descending ? -1 : 1;
  return [...tasks].sort((a, b) => {
    const group = compare(taskGroup(a, options.group), taskGroup(b, options.group));
    if (group) return options.group === "date" ? -group : group;
    const result = options.sort === "name" ? compare(a.fileName, b.fileName)
      : options.sort === "type" ? compare(taskType(a), taskType(b))
      : options.sort === "size" ? (a.total ?? -1) - (b.total ?? -1) : a.createdAt - b.createdAt;
    return direction * result || a.id.localeCompare(b.id);
  });
}
