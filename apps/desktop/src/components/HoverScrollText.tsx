import { useEffect, useRef } from "react";

const HOVER_DELAY_MS = 1_000;
const SCROLL_SPEED_PX_PER_SECOND = 40;
const MIN_TRAVEL_MS = 800;
const END_HOLD_MS = 650;

export type HoverScrollTextProps = {
  text: string;
  className?: string;
  innerClassName?: string;
};

export function HoverScrollText({ text, className, innerClassName }: HoverScrollTextProps) {
  const outerRef = useRef<HTMLSpanElement>(null);
  const innerRef = useRef<HTMLSpanElement>(null);
  const timerRef = useRef<number>();
  const animationRef = useRef<Animation>();

  function stop() {
    if (timerRef.current != null) window.clearTimeout(timerRef.current);
    timerRef.current = undefined;
    animationRef.current?.cancel();
    animationRef.current = undefined;
    innerRef.current?.classList.remove("hover-scroll-text-active");
    if (outerRef.current) outerRef.current.title = text;
  }

  function start() {
    stop();
    const outer = outerRef.current;
    const inner = innerRef.current;
    if (
      !outer ||
      !inner ||
      !window.matchMedia("(hover: hover) and (pointer: fine)").matches ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches ||
      inner.scrollWidth <= outer.clientWidth + 1
    ) return;

    outer.title = "";
    timerRef.current = window.setTimeout(() => {
      timerRef.current = undefined;
      inner.classList.add("hover-scroll-text-active");
      const distance = inner.scrollWidth - outer.clientWidth;
      if (distance <= 1) {
        stop();
        return;
      }

      const travelMs = Math.max(MIN_TRAVEL_MS, distance / SCROLL_SPEED_PX_PER_SECOND * 1_000);
      const duration = travelMs * 2 + END_HOLD_MS * 2;
      const endOffset = travelMs / duration;
      const endHoldOffset = (travelMs + END_HOLD_MS) / duration;
      const startOffset = (travelMs * 2 + END_HOLD_MS) / duration;
      animationRef.current = inner.animate([
        { transform: "translateX(0)", offset: 0 },
        { transform: `translateX(-${distance}px)`, offset: endOffset },
        { transform: `translateX(-${distance}px)`, offset: endHoldOffset },
        { transform: "translateX(0)", offset: startOffset },
        { transform: "translateX(0)", offset: 1 },
      ], {
        duration,
        easing: "linear",
        iterations: Infinity,
      });
    }, HOVER_DELAY_MS);
  }

  useEffect(() => stop, [text]);

  return (
    <span
      ref={outerRef}
      className={`hover-scroll-text${className ? ` ${className}` : ""}`}
      title={text}
      data-hover-scroll-text="true"
      onPointerEnter={start}
      onPointerLeave={stop}
    >
      <span
        ref={innerRef}
        className={`hover-scroll-text-inner${innerClassName ? ` ${innerClassName}` : ""}`}
        data-hover-scroll-inner="true"
      >
        {text}
      </span>
    </span>
  );
}
