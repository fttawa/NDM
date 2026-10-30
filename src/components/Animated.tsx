import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** 进度事件约每 500ms 一次，线性补间时长略长于间隔，使两次更新之间无停顿 */
export const TWEEN_MS = 600;
/** 平滑模式的时间常数：越大越稳，越小越跟手 */
const SMOOTH_TAU = 450;

/**
 * 数字动画：每帧直接写 DOM，不触发 React 重新渲染。
 * - linear：匀速补间到新值，适合单调递增的计数（已下载、百分比）
 * - smooth：指数平滑逼近新值，速度与方向连续变化，适合波动的数值（速度）
 */
export function AnimatedNumber({
  value,
  format,
  className,
  mode = "linear",
}: {
  value: number;
  format: (n: number) => string;
  className?: string;
  mode?: "linear" | "smooth";
}) {
  const ref = useRef<HTMLSpanElement>(null);
  const cur = useRef(value);
  const fmt = useRef(format);
  fmt.current = format;

  // 每次渲染同步一次文本（格式函数可能依赖其他属性，如总大小）
  useLayoutEffect(() => {
    if (ref.current) ref.current.textContent = fmt.current(cur.current);
  });

  useEffect(() => {
    const el = ref.current;
    const from = cur.current;
    if (!el || !isFinite(from) || !isFinite(value) || from === value) {
      cur.current = value;
      if (el) el.textContent = fmt.current(value);
      return;
    }
    let raf = 0;
    if (mode === "smooth") {
      let last = performance.now();
      const tick = (now: number) => {
        const dt = now - last;
        last = now;
        const diff = value - cur.current;
        cur.current += diff * (1 - Math.exp(-dt / SMOOTH_TAU));
        if (Math.abs(diff) < Math.max(1, Math.abs(value) * 1e-4)) cur.current = value;
        el.textContent = fmt.current(cur.current);
        if (cur.current !== value) raf = requestAnimationFrame(tick);
      };
      raf = requestAnimationFrame(tick);
    } else {
      const start = performance.now();
      const tick = (now: number) => {
        const t = Math.min(1, (now - start) / TWEEN_MS);
        cur.current = from + (value - from) * t;
        el.textContent = fmt.current(cur.current);
        if (t < 1) raf = requestAnimationFrame(tick);
      };
      raf = requestAnimationFrame(tick);
    }
    return () => cancelAnimationFrame(raf);
  }, [value, mode]);

  return <span ref={ref} className={className} />;
}

/** 数值平滑（会触发重新渲染，只用于少量组件，如图表纵轴） */
export function useTween(target: number): number {
  const [v, setV] = useState(target);
  const cur = useRef(target);
  useEffect(() => {
    if (cur.current === target) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = now - last;
      last = now;
      const diff = target - cur.current;
      cur.current += diff * (1 - Math.exp(-dt / SMOOTH_TAU));
      if (Math.abs(diff) < Math.abs(target) * 1e-3) cur.current = target;
      setV(cur.current);
      if (cur.current !== target) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [target]);
  return v;
}

/**
 * FLIP 布局动画：列表顺序或位置变化时，元素从旧位置平滑移动到新位置。
 * 使用 offsetLeft/offsetTop，不受页面滚动影响。
 */
export function useFlip(deps: unknown) {
  const nodes = useRef(new Map<string, HTMLElement>());
  const last = useRef(new Map<string, { x: number; y: number }>());

  useLayoutEffect(() => {
    const next = new Map<string, { x: number; y: number }>();
    nodes.current.forEach((el, id) => {
      const pos = { x: el.offsetLeft, y: el.offsetTop };
      next.set(id, pos);
      const prev = last.current.get(id);
      if (prev && (prev.x !== pos.x || prev.y !== pos.y)) {
        el.animate([{ transform: `translate(${prev.x - pos.x}px, ${prev.y - pos.y}px)` }, { transform: "none" }], {
          duration: 380,
          easing: "cubic-bezier(0.2, 0.8, 0.2, 1)",
        });
      }
    });
    last.current = next;
  }, [deps]);

  return (id: string) => (el: HTMLElement | null) => {
    if (el) nodes.current.set(id, el);
    else nodes.current.delete(id);
  };
}

const reducedMotion = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * 进出场：关闭后保留挂载 ms 毫秒，期间 closing 为 true，用于播放退场动画。
 */
export function usePresence(open: boolean, ms = 200) {
  const [mounted, setMounted] = useState(open);
  useEffect(() => {
    if (open) {
      setMounted(true);
      return;
    }
    const t = window.setTimeout(() => setMounted(false), reducedMotion() ? 0 : ms);
    return () => window.clearTimeout(t);
  }, [open, ms]);
  return { mounted: open || mounted, closing: !open && mounted };
}

/** 退场期间沿用最后一次有效的值（数据已清空时内容不至于先消失） */
export function useLatest<T>(value: T, keep: boolean): T {
  const ref = useRef(value);
  if (keep) ref.current = value;
  return keep ? value : ref.current;
}

/**
 * 滑动选中指示条：在容器上写入 --ind-x / --ind-w，由 CSS 的 ::after 跟随过渡。
 * 首次定位不播放过渡，避免从左侧滑入。
 */
export function useSlidingIndicator<T extends HTMLElement>(selector: string, deps: unknown) {
  const ref = useRef<T>(null);
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box) return;
    const place = () => {
      const el = box.querySelector<HTMLElement>(selector);
      box.classList.toggle("has-ind", !!el);
      if (!el) return;
      box.style.setProperty("--ind-x", `${el.offsetLeft}px`);
      box.style.setProperty("--ind-y", `${el.offsetTop}px`);
      box.style.setProperty("--ind-w", `${el.offsetWidth}px`);
      box.style.setProperty("--ind-h", `${el.offsetHeight}px`);
    };
    place();
    // 下一帧再开启过渡，首次渲染直接到位
    const raf = requestAnimationFrame(() => box.classList.add("ind-ready"));
    const ro = new ResizeObserver(place);
    ro.observe(box);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [deps, selector]);
  return ref;
}

/**
 * 列表退场：被移除的项在原位置保留 ms 毫秒并标记 leaving，之后真正移除（其余项再由 FLIP 补位）。
 * resetKey 变化（如切换筛选）时直接替换，不播放退场。
 */
export function useLeaving<T extends { id: string }>(items: T[], resetKey: unknown, ms = 220) {
  const shown = useRef<{ item: T; leaving: boolean }[]>([]);
  const lastKey = useRef(resetKey);
  const [, force] = useState(0);
  const timers = useRef(new Map<string, number>());

  useEffect(() => () => timers.current.forEach((t) => window.clearTimeout(t)), []);

  const ids = new Set(items.map((t) => t.id));
  let next: { item: T; leaving: boolean }[];
  if (lastKey.current !== resetKey || reducedMotion()) {
    lastKey.current = resetKey;
    next = items.map((item) => ({ item, leaving: false }));
  } else {
    next = items.map((item) => ({ item, leaving: false }));
    // 已消失的项插回原来的位置
    shown.current.forEach((entry, index) => {
      if (ids.has(entry.item.id)) return;
      next.splice(Math.min(index, next.length), 0, { item: entry.item, leaving: true });
    });
  }
  shown.current = next;

  useEffect(() => {
    next.forEach(({ item, leaving }) => {
      if (!leaving) {
        const t = timers.current.get(item.id);
        if (t) window.clearTimeout(t), timers.current.delete(item.id);
        return;
      }
      if (timers.current.has(item.id)) return;
      timers.current.set(
        item.id,
        window.setTimeout(() => {
          timers.current.delete(item.id);
          shown.current = shown.current.filter((e) => e.item.id !== item.id || !e.leaving);
          force((n) => n + 1);
        }, ms),
      );
    });
  });

  return next;
}
