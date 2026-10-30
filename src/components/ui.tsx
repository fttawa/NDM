import type { ReactNode } from "react";
import { useSlidingIndicator } from "./Animated";

export function Switch({ checked, onChange, label, disabled }: { checked: boolean; onChange: (v: boolean) => void; label?: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      disabled={disabled}
      className={`switch ${checked ? "on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

export function Slider({
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number) => void;
}) {
  const pct = ((value - min) / (max - min)) * 100;
  return (
    <input
      type="range"
      className="slider"
      min={min}
      max={max}
      step={step}
      value={value}
      style={{ ["--pct" as string]: `${pct}%` }}
      onChange={(e) => onChange(Number(e.target.value))}
    />
  );
}

export function IconButton({
  title,
  onClick,
  children,
  variant = "ghost",
  disabled,
}: {
  title: string;
  onClick?: (e: React.MouseEvent) => void;
  children: ReactNode;
  variant?: "ghost" | "danger" | "solid";
  disabled?: boolean;
}) {
  return (
    <button type="button" className={`icon-btn ${variant}`} title={title} aria-label={title} onClick={onClick} disabled={disabled}>
      {children}
    </button>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: ReactNode; count?: number }[];
  onChange: (v: T) => void;
}) {
  const ref = useSlidingIndicator<HTMLDivElement>("button.active", value);
  return (
    <div ref={ref} className="segmented glass">
      {options.map((o) => (
        <button key={o.value} type="button" className={o.value === value ? "active" : ""} onClick={() => onChange(o.value)}>
          {o.label}
          {o.count != null && o.count > 0 && <span className="count">{o.count}</span>}
        </button>
      ))}
    </div>
  );
}
