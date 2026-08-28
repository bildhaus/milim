// On-theme form controls - no native checkboxes/toggles/sliders/selects/color.
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Check, ChevronDown } from "./icons";

// ---- color math ----
const clamp01 = (n: number) => Math.max(0, Math.min(1, n));
const byte = (n: number) => Math.max(0, Math.min(255, Math.round(n)));
function rgbToHex(r: number, g: number, b: number): string {
  return "#" + [r, g, b].map((x) => byte(x).toString(16).padStart(2, "0")).join("");
}
function hexToRgb(hex: string): { r: number; g: number; b: number } {
  let h = hex.replace("#", "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h || "0", 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}
function rgbToHsv(r: number, g: number, b: number): { h: number; s: number; v: number } {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = 0;
  if (d) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
    if (h < 0) h += 360;
  }
  return { h, s: max ? d / max : 0, v: max };
}
function hsvToRgb(h: number, s: number, v: number): { r: number; g: number; b: number } {
  const c = v * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = v - c;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) [r, g] = [c, x];
  else if (h < 120) [r, g] = [x, c];
  else if (h < 180) [g, b] = [c, x];
  else if (h < 240) [g, b] = [x, c];
  else if (h < 300) [r, b] = [x, c];
  else [r, b] = [c, x];
  return { r: (r + m) * 255, g: (g + m) * 255, b: (b + m) * 255 };
}

function drag(onMove: (e: PointerEvent) => void) {
  const stop = () => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", stop);
    window.removeEventListener("pointercancel", stop);
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", stop);
  window.addEventListener("pointercancel", stop);
}

/** A themed color picker (swatch -> popover with SV square, hue slider, hex). */
export function ColorField({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (v: string) => void;
  label?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const { r, g, b } = hexToRgb(value);
  const { h, s, v } = rgbToHsv(r, g, b);
  const emit = (nh: number, ns: number, nv: number) => {
    const c = hsvToRgb(nh, ns, nv);
    onChange(rgbToHex(c.r, c.g, c.b));
  };

  return (
    <div className="ui-color" ref={ref}>
      <button type="button" className="ui-color-swatch" style={{ background: value }} title={value} aria-label={`Choose color, current value ${value}`} aria-expanded={open} onClick={() => setOpen((o) => !o)} />
      {label && <span className="ui-color-label">{label}</span>}
      {open && (
        <div className="ui-color-pop" role="dialog" aria-label="Color picker" onMouseDown={(e) => e.stopPropagation()}>
          <div
            className="ui-sv"
            role="slider"
            tabIndex={0}
            aria-label="Color saturation and brightness"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(s * 100)}
            aria-valuetext={`${Math.round(s * 100)}% saturation, ${Math.round(v * 100)}% brightness`}
            style={{ background: `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, hsl(${h}, 100%, 50%))` }}
            onPointerDown={(e) => {
              const el = e.currentTarget;
              const set = (cx: number, cy: number) => {
                const rect = el.getBoundingClientRect();
                emit(h, clamp01((cx - rect.left) / rect.width), 1 - clamp01((cy - rect.top) / rect.height));
              };
              set(e.clientX, e.clientY);
              drag((ev) => set(ev.clientX, ev.clientY));
            }}
            onKeyDown={(event) => {
              const delta = event.shiftKey ? 0.1 : 0.02;
              if (event.key === "ArrowLeft") emit(h, clamp01(s - delta), v);
              else if (event.key === "ArrowRight") emit(h, clamp01(s + delta), v);
              else if (event.key === "ArrowDown") emit(h, s, clamp01(v - delta));
              else if (event.key === "ArrowUp") emit(h, s, clamp01(v + delta));
              else return;
              event.preventDefault();
            }}
          >
            <span className="ui-sv-dot" style={{ left: `${s * 100}%`, top: `${(1 - v) * 100}%` }} />
          </div>
          <div
            className="ui-hue"
            role="slider"
            tabIndex={0}
            aria-label="Hue"
            aria-valuemin={0}
            aria-valuemax={360}
            aria-valuenow={Math.round(h)}
            onPointerDown={(e) => {
              const el = e.currentTarget;
              const set = (cx: number) => {
                const rect = el.getBoundingClientRect();
                emit(clamp01((cx - rect.left) / rect.width) * 360, s || 1, v || 1);
              };
              set(e.clientX);
              drag((ev) => set(ev.clientX));
            }}
            onKeyDown={(event) => {
              const delta = event.shiftKey ? 10 : 1;
              if (event.key === "ArrowLeft" || event.key === "ArrowDown") {
                emit((h - delta + 360) % 360, s || 1, v || 1);
              } else if (event.key === "ArrowRight" || event.key === "ArrowUp") {
                emit((h + delta) % 360, s || 1, v || 1);
              } else return;
              event.preventDefault();
            }}
          >
            <span className="ui-hue-thumb" style={{ left: `${(h / 360) * 100}%` }} />
          </div>
          <input
            className="ui-hex"
            aria-label="Hex color"
            value={value}
            onChange={(e) => {
              const x = e.target.value.trim();
              if (/^#?[0-9a-fA-F]{0,6}$/.test(x)) onChange(x.startsWith("#") ? x : "#" + x);
            }}
          />
        </div>
      )}
    </div>
  );
}

/** A pill switch (on = accent). */
export function Toggle({
  checked,
  onChange,
  label,
  ariaLabel,
  disabled = false,
  testId,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: ReactNode;
  ariaLabel?: string;
  disabled?: boolean;
  testId?: string;
}) {
  const sw = (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      data-testid={testId}
      className={"ui-switch" + (checked ? " on" : "")}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span className="ui-switch-knob" />
    </button>
  );
  if (!label) return sw;
  return (
    <label className="ui-toggle-row">
      {sw}
      <span>{label}</span>
    </label>
  );
}

/** A custom checkbox (box + check). */
export function Checkbox({
  checked,
  onChange,
  children,
  title,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  children?: ReactNode;
  title?: string;
}) {
  return (
    <label className="ui-check" title={title}>
      <input
        className="ui-check-native"
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      <span className={"ui-check-box" + (checked ? " on" : "")} aria-hidden="true">
        {checked && <Check size={11} />}
      </span>
      {children && <span>{children}</span>}
    </label>
  );
}

/** A pointer/keyboard-driven slider (no native range). */
export function Slider({
  min,
  max,
  step,
  value,
  onChange,
  ariaLabel,
}: {
  min: number;
  max: number;
  step: number;
  value: number;
  onChange: (v: number) => void;
  ariaLabel?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const pct = max > min ? ((value - min) / (max - min)) * 100 : 0;

  const clamp = (v: number) => Math.max(min, Math.min(max, v));
  const fromX = (clientX: number) => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const r = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
    const raw = min + r * (max - min);
    onChange(clamp(Math.round(raw / step) * step));
  };

  const onPointerDown = (e: React.PointerEvent) => {
    e.preventDefault();
    fromX(e.clientX);
    const move = (ev: PointerEvent) => fromX(ev.clientX);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  return (
    <div
      className="ui-slider"
      ref={ref}
      role="slider"
      aria-label={ariaLabel}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={(e) => {
        if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
          e.preventDefault();
          onChange(clamp(value - step));
        }
        if (e.key === "ArrowRight" || e.key === "ArrowUp") {
          e.preventDefault();
          onChange(clamp(value + step));
        }
      }}
    >
      <div className="ui-slider-track">
        <div className="ui-slider-fill" style={{ width: `${pct}%` }} />
      </div>
      <div className="ui-slider-thumb" style={{ left: `${pct}%` }} />
    </div>
  );
}

export interface Option {
  value: string;
  label: string;
  leading?: ReactNode;
}

/** A themed dropdown (no native select). */
export function Select({
  value,
  options,
  onChange,
  placeholder = "Select...",
  ariaLabel,
  testId,
}: {
  value: string;
  options: Option[];
  onChange: (v: string) => void;
  placeholder?: string;
  ariaLabel?: string;
  testId?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const pendingFocusIndex = useRef<number | null>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  useEffect(() => {
    if (!open || pendingFocusIndex.current === null) return;
    optionRefs.current[pendingFocusIndex.current]?.focus();
    pendingFocusIndex.current = null;
  }, [open]);

  function focusOption(index: number) {
    if (!options.length) return;
    const next = Math.max(0, Math.min(options.length - 1, index));
    if (open) optionRefs.current[next]?.focus();
    else {
      pendingFocusIndex.current = next;
      setOpen(true);
    }
  }

  function closeAndFocusTrigger() {
    setOpen(false);
    window.requestAnimationFrame(() => triggerRef.current?.focus());
  }

  const current = options.find((o) => o.value === value);
  const currentIndex = Math.max(0, options.findIndex((option) => option.value === value));
  return (
    <div
      className="ui-select"
      ref={ref}
      onKeyDown={(event) => {
        if (!open) return;
        if (event.key === "Tab") {
          setOpen(false);
          return;
        }
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          closeAndFocusTrigger();
          return;
        }
        const index = optionRefs.current.indexOf(event.target as HTMLButtonElement);
        if (index < 0) return;
        if (event.key === "ArrowDown") focusOption((index + 1) % options.length);
        else if (event.key === "ArrowUp") focusOption((index - 1 + options.length) % options.length);
        else if (event.key === "Home") focusOption(0);
        else if (event.key === "End") focusOption(options.length - 1);
        else return;
        event.preventDefault();
      }}
    >
      <button
        ref={triggerRef}
        type="button"
        className="ui-select-btn"
        data-testid={testId}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={menuId}
        onClick={() => setOpen((next) => !next)}
        onKeyDown={(event) => {
          if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
          event.preventDefault();
          if (event.key === "ArrowDown") focusOption(currentIndex);
          else if (event.key === "ArrowUp") focusOption(options.findIndex((option) => option.value === value) >= 0 ? currentIndex : options.length - 1);
          else focusOption(event.key === "Home" ? 0 : options.length - 1);
        }}
      >
        <span className={"ui-select-value" + (current ? "" : " placeholder")}>
          {current?.leading}
          <span>{current?.label ?? placeholder}</span>
        </span>
        <ChevronDown size={13} />
      </button>
      {open && (
        <div className="ui-select-menu" id={menuId} role="listbox" aria-label={ariaLabel ?? placeholder}>
          {options.map((o, index) => (
            <button
              key={o.value}
              ref={(node) => {
                optionRefs.current[index] = node;
              }}
              type="button"
              className={"ui-select-item" + (o.value === value ? " active" : "")}
              role="option"
              aria-selected={o.value === value}
              tabIndex={-1}
              onClick={() => {
                onChange(o.value);
                closeAndFocusTrigger();
              }}
            >
              {o.leading}
              <span>{o.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
