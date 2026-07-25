import { useEffect, useRef } from "react";

const TOKENS = [
  "local",
  "privacy",
  "memory",
  "agent",
  "context",
  "redact",
  "model",
  "thread",
  "shell",
  "voice",
  "session",
  "offline",
  "tool",
  "mcp",
];
const FONT_SIZE = 15;
const LINE_HEIGHT = 18;
const CELL = 18;

function hash(seed: number) {
  const s = Math.sin(seed * 127.1) * 43758.5453;
  return s - Math.floor(s);
}

export function HeroAsciiField() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    const host = canvas?.parentElement;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !host || !ctx) return;

    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let frame = 0;
    let last = 0;
    let dpr = 1;
    let visible = true;

    const resize = () => {
      const rect = host.getBoundingClientRect();
      dpr = Math.min(window.devicePixelRatio || 1, 1.8);
      canvas.width = Math.max(1, Math.floor(rect.width * dpr));
      canvas.height = Math.max(1, Math.floor(rect.height * dpr));
      canvas.style.width = `${rect.width}px`;
      canvas.style.height = `${rect.height}px`;
    };

    const drawShaderTexture = (now: number, light: boolean) => {
      const w = canvas.width;
      const h = canvas.height;
      const cell = CELL * dpr;
      const phase = reduceMotion ? 0 : Math.floor(now / 360);
      const drift = reduceMotion ? 0 : (now / 58) % (cell * 2);

      ctx.save();
      ctx.globalCompositeOperation = "source-atop";
      ctx.translate(-drift, 0);

      for (let y = -cell; y < h + cell; y += cell) {
        const row = Math.floor(y / cell);
        for (let x = -cell; x < w + cell * 2; x += cell) {
          const col = Math.floor(x / cell);
          const wave = hash(row * 61 + col * 17 + phase * 29);
          const bright = (row + col + phase) % 2 === 0;
          const alpha = bright ? 0.46 + wave * 0.2 : 0.14 + wave * 0.12;
          ctx.fillStyle = light
            ? bright
              ? `rgba(36,36,36,${alpha * 0.34})`
              : `rgba(96,96,96,${alpha * 0.22})`
            : bright
              ? `rgba(198,198,198,${alpha})`
              : `rgba(112,112,112,${alpha})`;
          ctx.fillRect(x, y, cell, cell);
        }
      }

      ctx.restore();
      ctx.save();
      ctx.globalCompositeOperation = "source-atop";
      const beam = ctx.createLinearGradient(0, 0, w, h);
      if (light) {
        beam.addColorStop(0, "rgba(255,255,255,0)");
        beam.addColorStop(0.45, "rgba(24,24,24,0.12)");
        beam.addColorStop(1, "rgba(255,255,255,0)");
      } else {
        beam.addColorStop(0, "rgba(255,255,255,0)");
        beam.addColorStop(0.44, "rgba(210,210,210,0.18)");
        beam.addColorStop(0.58, "rgba(150,150,150,0.2)");
        beam.addColorStop(1, "rgba(255,255,255,0)");
      }
      ctx.fillStyle = beam;
      ctx.fillRect(0, 0, w, h);
      ctx.restore();
    };

    const drawTextMask = (now: number) => {
      const w = canvas.width;
      const h = canvas.height;
      const driftTick = reduceMotion ? 0 : Math.floor(now / 420);
      const lineHeight = LINE_HEIGHT * dpr;
      const rows = Math.ceil(h / lineHeight) + 4;

      ctx.globalCompositeOperation = "source-over";
      ctx.font = `${Math.round(FONT_SIZE * dpr)}px "JetBrains Mono", ui-monospace, monospace`;
      ctx.textBaseline = "top";
      ctx.textAlign = "left";
      ctx.fillStyle = "rgba(255,255,255,1)";

      for (let row = -1; row < rows; row++) {
        const y = row * lineHeight;
        let x = -((driftTick * 11 + row * 29) % Math.round(132 * dpr));
        let col = 0;

        while (x < w + 120 * dpr) {
          const seed = row * 101 + col * 47;
          const glyphTick = reduceMotion ? 0 : Math.floor((now + hash(seed) * 1800) / (260 + hash(seed + 17) * 960));
          const opacityTick = reduceMotion
            ? 0
            : Math.floor((now + hash(seed + 31) * 1200) / (420 + hash(seed + 43) * 1200));
          const h1 = hash(seed + glyphTick * 37);
          const token = TOKENS[Math.floor(h1 * TOKENS.length) % TOKENS.length];
          const text = h1 > 0.74 ? `${token}${Math.floor(hash(h1 * 911) * 10)}` : token;

          ctx.globalAlpha = 0.18 + hash(seed * 13 + opacityTick * 79) * 0.62;
          ctx.fillText(text, x, y);
          x += ctx.measureText(text).width;
          col++;
        }
      }

      ctx.globalAlpha = 1;
      ctx.globalCompositeOperation = "source-over";
    };

    const boostMaskedTexture = (light: boolean) => {
      const w = canvas.width;
      const h = canvas.height;
      const glow = ctx.createLinearGradient(0, 0, 0, h);

      if (light) {
        glow.addColorStop(0, "rgba(24,24,24,0.04)");
        glow.addColorStop(0.46, "rgba(24,24,24,0.22)");
        glow.addColorStop(1, "rgba(24,24,24,0.05)");
      } else {
        glow.addColorStop(0, "rgba(110,110,110,0.12)");
        glow.addColorStop(0.36, "rgba(190,190,190,0.34)");
        glow.addColorStop(0.52, "rgba(235,235,235,0.22)");
        glow.addColorStop(1, "rgba(120,120,120,0.1)");
      }

      ctx.save();
      ctx.globalCompositeOperation = "source-atop";
      ctx.fillStyle = glow;
      ctx.fillRect(0, 0, w, h);
      ctx.restore();
    };

    const drawShaderText = (now = 0) => {
      const theme = document.documentElement.dataset.theme;
      const systemLight = window.matchMedia("(prefers-color-scheme: light)").matches;
      const light = theme === "light" || (theme === "system" && systemLight);
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      drawTextMask(now);
      drawShaderTexture(now, light);
      boostMaskedTexture(light);
    };

    const loop = (now: number) => {
      if (!visible) return;
      frame = requestAnimationFrame(loop);
      if (now - last < 50) return;
      last = now;
      drawShaderText(now);
    };

    const onResize = () => {
      resize();
      drawShaderText();
    };

    resize();
    drawShaderText();
    window.addEventListener("resize", onResize);
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      cancelAnimationFrame(frame);
      if (visible && !reduceMotion) frame = requestAnimationFrame(loop);
    });
    observer.observe(canvas);
    if (!reduceMotion) {
      frame = requestAnimationFrame(loop);
    }

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", onResize);
    };
  }, []);

  return <canvas className="hero-ascii-field" aria-hidden="true" ref={ref} />;
}
