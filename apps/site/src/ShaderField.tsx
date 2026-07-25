import { useEffect, useRef } from "react";
import { Mesh, Program, Renderer, Triangle } from "ogl";

const vertex = `
attribute vec2 uv;
attribute vec2 position;
varying vec2 vUv;

void main() {
  vUv = uv;
  gl_Position = vec4(position, 0.0, 1.0);
}
`;

const fragment = `
precision highp float;

uniform float uTime;
uniform float uScroll;
uniform float uDpr;
uniform vec2 uPointer;
uniform vec2 uResolution;
uniform float uDither;
varying vec2 vUv;
const float CHECKER_CELL = 94.0;
const float DITHER_GRID = 3.0;

float hash(vec2 p) {
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float noise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);

  return mix(
    mix(hash(i + vec2(0.0, 0.0)), hash(i + vec2(1.0, 0.0)), u.x),
    mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x),
    u.y
  );
}

float band(vec2 uv, float offset, float width) {
  float y = uv.y + sin(uv.x * 2.6 + uTime * 0.18 + offset) * 0.08;
  return smoothstep(width, 0.0, abs(y - offset));
}

float bayer2(vec2 p) {
  vec2 q = floor(mod(p, 2.0));
  if (q.y < 0.5) {
    return q.x < 0.5 ? 0.0 : 2.0;
  }
  return q.x < 0.5 ? 3.0 : 1.0;
}

float bayer4(vec2 p) {
  vec2 q = floor(mod(p, 4.0));
  return (bayer2(q) * 4.0 + bayer2(floor(q * 0.5))) / 16.0;
}

void main() {
  vec2 cssResolution = uResolution / uDpr;
  vec2 rawUv = vUv;
  if (uDither > 0.5) {
    rawUv = (floor(vUv * cssResolution / DITHER_GRID) + 0.5) * DITHER_GRID / cssResolution;
  }
  vec2 pointerDelta = rawUv - uPointer;
  float cursor = smoothstep(0.38, 0.0, length(pointerDelta));
  vec2 uv = rawUv + pointerDelta * cursor * 0.035;
  vec2 p = (uv - 0.5) * vec2(uResolution.x / uResolution.y, 1.0);

  float time = uTime * 0.08;
  float field = 0.0;
  field += band(uv, 0.22 + uScroll * 0.08, 0.085) * 0.42;
  field += band(uv, 0.58 - uScroll * 0.05, 0.11) * 0.34;
  field += band(uv, 0.81 + sin(time) * 0.06, 0.07) * 0.22;
  field += cursor * 0.16;

  vec2 boardUv = (uv - 0.5) * cssResolution;
  boardUv = mat2(0.70710678, -0.70710678, 0.70710678, 0.70710678) * boardUv;
  vec2 board = floor(boardUv / CHECKER_CELL + vec2(time * 0.935, -time * 0.595));
  float checker = mod(board.x + board.y, 2.0);
  float cell = hash(board);
  float checkerGlow = checker * (0.03 + cell * 0.015);
  float bitplane = checkerGlow + cursor * 0.08;

  float n = noise(p * 5.0 + vec2(time * 2.0, -time));
  float ridge = smoothstep(0.25, 0.9, 1.0 - abs(p.y + sin(p.x * 3.8 + time) * 0.22));
  float vignette = smoothstep(0.88, 0.15, length(p));
  float scan = 0.96 + sin(uv.y * uResolution.y * 1.12) * 0.025;

  vec3 graphite = vec3(0.015, 0.016, 0.017);
  vec3 paper = vec3(0.92, 0.91, 0.86);
  vec3 cyan = vec3(0.45, 0.62, 0.68);
  vec3 moss = vec3(0.62, 0.70, 0.55);

  float signal = (field + bitplane + ridge * 0.16 + n * 0.075) * vignette * scan;
  float ordered = 1.0;
  if (uDither > 0.5) {
    float threshold = bayer4(floor(gl_FragCoord.xy / max(1.0, DITHER_GRID * uDpr)));
    float tone = clamp(signal * 1.65 + checkerGlow * 2.4 + cursor * 0.2, 0.0, 1.0);
    ordered = step(threshold, tone);
    signal *= mix(0.42, 1.34, ordered);
    field *= mix(0.74, 1.12, ordered);
    checkerGlow *= mix(0.4, 1.45, ordered);
  }
  vec3 color = graphite + paper * signal * 0.68 + cyan * field * 0.2 + moss * checkerGlow * 0.36 + paper * cursor * 0.12;

  float alphaDither = mix(1.0, mix(0.72, 1.08, ordered), step(0.5, uDither));
  float alpha = clamp((signal * 1.18 + checker * vignette * 0.07 + (1.0 - checker) * vignette * 0.04 + cursor * 0.1) * alphaDither, 0.0, 0.68);
  gl_FragColor = vec4(color, alpha);
}
`;

type ShaderFieldProps = {
  dither?: boolean;
};

export function ShaderField({ dither = true }: ShaderFieldProps) {
  const container = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = container.current;
    if (!host) return;

    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const dpr = Math.min(window.devicePixelRatio || 1, 1.7);
    const renderer = new Renderer({
      alpha: true,
      antialias: false,
      dpr,
    });
    const gl = renderer.gl;
    gl.canvas.className = "shader-canvas";
    gl.clearColor(0, 0, 0, 0);
    host.appendChild(gl.canvas);

    const geometry = new Triangle(gl);
    const program = new Program(gl, {
      vertex,
      fragment,
      transparent: true,
      uniforms: {
        uTime: { value: 0 },
        uScroll: { value: 0 },
        uDpr: { value: dpr },
        uDither: { value: dither ? 1 : 0 },
        uPointer: { value: [0.5, 0.5] },
        uResolution: { value: [1, 1] },
      },
    });
    const mesh = new Mesh(gl, { geometry, program });

    let frame = 0;
    let start = performance.now();
    let visible = true;
    let scrollProgress = 0;

    const resize = () => {
      const rect = host.getBoundingClientRect();
      const width = Math.max(1, Math.floor(rect.width));
      const height = Math.max(1, Math.floor(rect.height));
      renderer.setSize(width, height);
      program.uniforms.uResolution.value = [gl.canvas.width, gl.canvas.height];
    };

    const updatePointer = (event: PointerEvent) => {
      const rect = host.getBoundingClientRect();
      program.uniforms.uPointer.value = [
        (event.clientX - rect.left) / Math.max(1, rect.width),
        1 - (event.clientY - rect.top) / Math.max(1, rect.height),
      ];
    };

    const updateScroll = () => {
      scrollProgress = Math.min(1, window.scrollY / Math.max(1, window.innerHeight));
    };

    const render = (now = 0) => {
      const elapsed = reduceMotion ? 0 : (now - start) * 0.001;
      program.uniforms.uTime.value = elapsed;
      program.uniforms.uScroll.value = scrollProgress;
      renderer.render({ scene: mesh });
    };

    const loop = (now: number) => {
      if (!visible) return;
      render(now);
      frame = requestAnimationFrame(loop);
    };

    const onResize = () => {
      resize();
      render(performance.now());
    };

    resize();
    updateScroll();
    render(performance.now());
    window.addEventListener("resize", onResize);
    window.addEventListener("scroll", updateScroll, { passive: true });
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      cancelAnimationFrame(frame);
      if (visible && !reduceMotion) frame = requestAnimationFrame(loop);
    });
    observer.observe(host);
    if (!reduceMotion) {
      frame = requestAnimationFrame(loop);
    }
    if (!reduceMotion && window.matchMedia("(hover: hover) and (pointer: fine)").matches) {
      window.addEventListener("pointermove", updatePointer);
    }

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", onResize);
      window.removeEventListener("scroll", updateScroll);
      window.removeEventListener("pointermove", updatePointer);
      start = 0;
      gl.canvas.remove();
    };
  }, [dither]);

  return <div className={`shader-field${dither ? " shader-field-dither" : ""}`} aria-hidden="true" ref={container} />;
}
