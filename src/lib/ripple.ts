/**
 * MD3 按压涟漪：仅在 Material 3 主题下生效。
 * 按下时在元素上记录按压点和最远角距离，由 theme-md3.css 中的径向渐变从按压点扩散并淡出。
 * 不插入额外节点，不改变元素定位。
 */
const TARGETS = [
  ".btn",
  ".icon-btn",
  ".rail-fab",
  ".chip-btn",
  ".segmented button",
  ".download-filters button",
  ".view-options-choices button",
  ".task-card",
  ".tile-stat-link",
  ".tile-paste",
  ".schedule-task",
  ".home-row",
  ".home-more",
].join(",");

export function installRipple() {
  document.addEventListener(
    "pointerdown",
    (e) => {
      if (e.button !== 0 || document.documentElement.dataset.theme !== "md3") return;
      if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
      const el = (e.target as Element | null)?.closest<HTMLElement>(TARGETS);
      if (!el || (el as HTMLButtonElement).disabled) return;
      // 卡片上的按钮自己出涟漪，卡片本身不重复
      if (el.classList.contains("task-card") && (e.target as Element).closest("button")) return;
      const r = el.getBoundingClientRect();
      const x = e.clientX - r.left;
      const y = e.clientY - r.top;
      const max = Math.hypot(Math.max(x, r.width - x), Math.max(y, r.height - y));
      el.style.setProperty("--rx", `${x}px`);
      el.style.setProperty("--ry", `${y}px`);
      // 用 WAAPI 只驱动两个已注册的自定义属性，不覆盖元素自身的 CSS 动画与过渡
      el.animate(
        [
          { "--ripple-r": "0px", "--ripple-o": "14%" },
          { "--ripple-r": `${max}px`, "--ripple-o": "10%", offset: 0.6 },
          { "--ripple-r": `${max}px`, "--ripple-o": "0%" },
        ] as Keyframe[],
        { duration: 550, easing: "cubic-bezier(0.2, 0, 0, 1)" },
      );
    },
    { passive: true },
  );
}
