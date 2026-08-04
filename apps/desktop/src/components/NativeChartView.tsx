import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

type ChartType = "bar" | "line" | "pie" | "scatter";
type BarOrientation = "vertical" | "horizontal";

export type ChartNumberFormat = {
  style: "number" | "percent" | "currency";
  currency?: string;
  precision?: number;
  notation?: "standard" | "compact";
  sign_display?: "auto" | "always";
};

type ChartPoint = { x: string | number; y: number };
type ChartSeries = { name: string; points: ChartPoint[] };

export type NativeChartSpec = {
  title: string;
  subtitle?: string;
  type: ChartType;
  orientation?: BarOrientation;
  x_label?: string;
  y_label?: string;
  x_format?: ChartNumberFormat;
  y_format?: ChartNumberFormat;
  series: ChartSeries[];
};

type NativeChartViewProps = {
  argumentsText?: string;
  result?: unknown;
  status?: "running" | "done" | "error";
};

type TooltipRow = {
  entryIndex: number;
  label: string;
  value: string;
  color: string;
};

type ActiveMark = {
  entryIndex: number;
  pointIndex: number;
  categoryIndex?: number;
  svgX: number;
  svgY: number;
  left: number;
  top: number;
  heading: string;
  rows: TooltipRow[];
  placement: "above" | "below";
};

type ChartInteractionProps = {
  activeMark: ActiveMark | null;
  hiddenEntries: ReadonlySet<number>;
  pinned: boolean;
  onActivate: (mark: ActiveMark, pin?: boolean) => void;
  onClear: () => void;
};

const COLORS = [
  "var(--chart-series-1)",
  "var(--chart-series-2)",
  "var(--chart-series-3)",
  "var(--chart-series-4)",
  "var(--chart-series-5)",
  "var(--chart-series-6)",
  "var(--chart-series-7)",
  "var(--chart-series-8)",
];

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function hasOnlyKeys(value: Record<string, unknown>, allowed: readonly string[]): boolean {
  return Object.keys(value).every((key) => allowed.includes(key));
}

function boundedText(value: unknown, max: number, optional = false): string | undefined {
  if (typeof value !== "string" || value.length > max || (!optional && !value.trim())) return undefined;
  return value;
}

function parseNumberFormat(value: unknown): ChartNumberFormat | null {
  const source = record(value);
  if (!source || !hasOnlyKeys(source, ["style", "currency", "precision", "notation", "sign_display"])) return null;
  const style = source.style;
  const currency = source.currency;
  const precision = source.precision;
  const notation = source.notation;
  const signDisplay = source.sign_display;
  if (!['number', 'percent', 'currency'].includes(String(style))) return null;
  if (precision !== undefined && (!Number.isInteger(precision) || Number(precision) < 0 || Number(precision) > 4)) return null;
  if (notation !== undefined && !["standard", "compact"].includes(String(notation))) return null;
  if (signDisplay !== undefined && !["auto", "always"].includes(String(signDisplay))) return null;
  if (style === "currency") {
    if (typeof currency !== "string" || !/^[A-Z]{3}$/.test(currency)) return null;
  } else if (currency !== undefined) {
    return null;
  }
  return {
    style: style as ChartNumberFormat["style"],
    currency: currency as string | undefined,
    precision: precision as number | undefined,
    notation: notation as ChartNumberFormat["notation"],
    sign_display: signDisplay as ChartNumberFormat["sign_display"],
  };
}

export function parseNativeChartSpec(value: unknown): NativeChartSpec | null {
  const source = record(value);
  if (!source || !hasOnlyKeys(source, ["title", "subtitle", "type", "orientation", "x_label", "y_label", "x_format", "y_format", "series"])) return null;
  const title = boundedText(source.title, 160);
  const type = source.type;
  const orientation = source.orientation;
  const rawSeries = source.series;
  if (!title || !["bar", "line", "pie", "scatter"].includes(String(type)) || !Array.isArray(rawSeries) || rawSeries.length < 1 || rawSeries.length > 8) return null;
  if (orientation !== undefined && (!["vertical", "horizontal"].includes(String(orientation)) || type !== "bar")) return null;
  const xFormat = source.x_format === undefined ? undefined : parseNumberFormat(source.x_format);
  const yFormat = source.y_format === undefined ? undefined : parseNumberFormat(source.y_format);
  if ((source.x_format !== undefined && !xFormat) || (source.y_format !== undefined && !yFormat) || (xFormat && type !== "scatter")) return null;
  const series: ChartSeries[] = [];
  let pointCount = 0;
  for (const raw of rawSeries) {
    const item = record(raw);
    if (!item || !hasOnlyKeys(item, ["name", "points"])) return null;
    const name = boundedText(item.name, 80);
    if (!name || !Array.isArray(item.points) || item.points.length < 1) return null;
    const points: ChartPoint[] = [];
    for (const rawPoint of item.points) {
      const point = record(rawPoint);
      if (!point || !hasOnlyKeys(point, ["x", "y"])) return null;
      const x = point.x;
      const y = point.y;
      if ((typeof x !== "string" && typeof x !== "number") || (typeof x === "string" && (!x.trim() || x.length > 80)) || (typeof x === "number" && !Number.isFinite(x)) || typeof y !== "number" || !Number.isFinite(y)) return null;
      if (type === "scatter" && typeof x !== "number") return null;
      points.push({ x, y });
      pointCount += 1;
    }
    series.push({ name, points });
  }
  if (pointCount > 400 || (type === "pie" && (series.length !== 1 || series[0].points.length > 12 || series[0].points.some((point) => point.y < 0) || series[0].points.reduce((sum, point) => sum + point.y, 0) <= 0))) return null;
  return {
    title,
    type: type as ChartType,
    orientation: orientation as BarOrientation | undefined,
    series,
    subtitle: boundedText(source.subtitle, 300, true),
    x_label: boundedText(source.x_label, 80, true),
    y_label: boundedText(source.y_label, 80, true),
    x_format: xFormat || undefined,
    y_format: yFormat || undefined,
  };
}

function specFromArguments(value?: string): NativeChartSpec | null {
  if (!value?.trim()) return null;
  try {
    return parseNativeChartSpec(JSON.parse(value));
  } catch {
    return null;
  }
}

export function formatChartValue(value: number, format?: ChartNumberFormat): string {
  const normalized = Object.is(value, -0) ? 0 : value;
  const options: Intl.NumberFormatOptions = {
    notation: format?.notation ?? "standard",
    signDisplay: format?.sign_display ?? "auto",
  };
  if (format?.style === "currency") {
    options.style = "currency";
    options.currency = format.currency;
    options.currencyDisplay = "narrowSymbol";
  }
  if (format?.precision !== undefined) {
    options.minimumFractionDigits = format.precision;
    options.maximumFractionDigits = format.precision;
  } else if (format?.notation === "compact") {
    options.maximumFractionDigits = 1;
  } else if (format?.style !== "currency") {
    options.maximumFractionDigits = 2;
  }
  const formatted = new Intl.NumberFormat(undefined, options).format(normalized);
  return format?.style === "percent" ? `${formatted}%` : formatted;
}

export function niceChartTicks(minimum: number, maximum: number, target = 5): number[] {
  let min = Math.min(minimum, maximum);
  let max = Math.max(minimum, maximum);
  if (min === max) {
    const padding = Math.max(Math.abs(min) * 0.1, 1);
    min -= padding;
    max += padding;
  }
  const roughStep = (max - min) / Math.max(1, target - 1);
  const magnitude = 10 ** Math.floor(Math.log10(roughStep));
  const fraction = roughStep / magnitude;
  const step = (fraction <= 1.5 ? 1 : fraction <= 3 ? 2 : fraction <= 7 ? 5 : 10) * magnitude;
  const start = Math.floor(min / step) * step;
  const end = Math.ceil(max / step) * step;
  const ticks: number[] = [];
  for (let value = start; value <= end + step / 2 && ticks.length < 20; value += step) {
    ticks.push(Number(value.toPrecision(12)));
  }
  return ticks;
}

export function chartCategoryTickIndexes(labels: string[], plotWidth: number): number[] {
  if (labels.length <= 2) return labels.map((_, index) => index);
  const widest = Math.min(96, Math.max(42, ...labels.map((label) => label.length * 5.8)));
  const maxTicks = Math.max(2, Math.floor(plotWidth / (widest + 12)));
  if (labels.length <= maxTicks) return labels.map((_, index) => index);
  const step = Math.ceil((labels.length - 1) / (maxTicks - 1));
  const indexes = [0];
  for (let index = step; index < labels.length - 1; index += step) indexes.push(index);
  indexes.push(labels.length - 1);
  return indexes;
}

export function chartNumericTickIndexes(ticks: number[], formattedTicks: string[], plotWidth: number): number[] {
  if (ticks.length <= 2) return ticks.map((_, index) => index);
  const first = ticks[0];
  const last = ticks[ticks.length - 1];
  const span = last - first || 1;
  const position = (index: number) => ((ticks[index] - first) / span) * plotWidth;
  const minimumGap = Math.min(96, Math.max(38, ...formattedTicks.map((tick) => tick.length * 6 + 10)));
  const zeroIndex = ticks.findIndex((tick) => Math.abs(tick) < Number.EPSILON);
  const priority = [0, ticks.length - 1, ...(zeroIndex > 0 && zeroIndex < ticks.length - 1 ? [zeroIndex] : []), ...ticks.map((_, index) => index)];
  const selected: number[] = [];
  for (const index of priority) {
    if (!selected.includes(index) && (selected.length < 2 || selected.every((other) => Math.abs(position(index) - position(other)) >= minimumGap))) selected.push(index);
  }
  return selected.sort((a, b) => a - b);
}

function splitCategoryLabel(label: string, maxCharacters: number): string[] {
  if (label.length <= maxCharacters) return [label];
  const firstWindow = label.slice(0, maxCharacters + 1);
  const splitAt = Math.max(firstWindow.lastIndexOf(" "), firstWindow.lastIndexOf("–"), firstWindow.lastIndexOf("-"));
  const boundary = splitAt >= Math.floor(maxCharacters * 0.55) ? splitAt + 1 : maxCharacters;
  const first = label.slice(0, boundary).trim();
  const rest = label.slice(boundary).trim();
  return [first, rest.length <= maxCharacters ? rest : `${rest.slice(0, Math.max(1, maxCharacters - 3))}...`];
}

function chartDescription(spec: NativeChartSpec): string {
  return spec.series
    .map((series) => `${series.name}: ${series.points.map((point) => `${typeof point.x === "number" ? formatChartValue(point.x, spec.x_format) : point.x}, ${formatChartValue(point.y, spec.y_format)}`).join("; ")}`)
    .join(". ");
}

function scale(min: number, max: number, start: number, length: number) {
  const span = max - min || 1;
  return (value: number) => start + ((value - min) / span) * length;
}

type BarFreeEnd = "top" | "bottom" | "left" | "right";

export function barMarkPath(x: number, y: number, width: number, height: number, radius: number, freeEnd: BarFreeEnd): string {
  const right = x + width;
  const bottom = y + height;
  const r = Math.min(radius, width / 2, height / 2);
  if (freeEnd === "top") return `M ${x} ${y + r} Q ${x} ${y} ${x + r} ${y} H ${right - r} Q ${right} ${y} ${right} ${y + r} V ${bottom} H ${x} Z`;
  if (freeEnd === "bottom") return `M ${x} ${y} H ${right} V ${bottom - r} Q ${right} ${bottom} ${right - r} ${bottom} H ${x + r} Q ${x} ${bottom} ${x} ${bottom - r} V ${y} Z`;
  if (freeEnd === "left") return `M ${x + r} ${y} H ${right} V ${bottom} H ${x + r} Q ${x} ${bottom} ${x} ${bottom - r} V ${y + r} Q ${x} ${y} ${x + r} ${y} Z`;
  return `M ${x} ${y} H ${right - r} Q ${right} ${y} ${right} ${y + r} V ${bottom - r} Q ${right} ${bottom} ${right - r} ${bottom} H ${x} Z`;
}

function chartDomain(values: number[], forceZero: boolean, reserveLabelSpace = false): { min: number; max: number; ticks: number[] } {
  let min = Math.min(...values);
  let max = Math.max(...values);
  if (forceZero) {
    min = Math.min(0, min);
    max = Math.max(0, max);
  }
  if (min === max) {
    const padding = Math.max(Math.abs(min) * 0.1, 1);
    min -= padding;
    max += padding;
  } else if (forceZero) {
    const padding = (max - min) * (reserveLabelSpace ? 0.1 : 0.02);
    if (min < 0) min -= padding;
    if (max > 0) max += padding;
  } else {
    const padding = (max - min) * 0.08;
    min -= padding;
    max += padding;
  }
  const ticks = niceChartTicks(min, max);
  return { min: ticks[0], max: ticks[ticks.length - 1], ticks };
}

function markClass(activeMark: ActiveMark | null, entryIndex: number, pointIndex: number, categoryIndex?: number): string {
  if (!activeMark) return "native-chart-mark";
  if (activeMark.entryIndex === entryIndex && activeMark.pointIndex === pointIndex) return "native-chart-mark active";
  if (categoryIndex !== undefined && activeMark.categoryIndex === categoryIndex) return "native-chart-mark related";
  return "native-chart-mark dimmed";
}

function navigateChartMarks(event: KeyboardEvent<SVGElement>) {
  if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
  const svg = event.currentTarget.ownerSVGElement;
  const marks = svg ? Array.from(svg.querySelectorAll<SVGElement>("[data-chart-mark]")) : [];
  if (!marks.length) return;
  const current = event.currentTarget;
  const currentSeries = current.dataset.seriesIndex;
  const currentPoint = current.dataset.pointIndex;
  const currentCategory = current.dataset.categoryIndex;
  const horizontalBar = svg?.dataset.chartType === "bar" && svg.dataset.orientation === "horizontal";
  let candidates: SVGElement[] = [];
  let direction = 0;
  if (svg?.dataset.chartType === "pie") {
    candidates = marks;
    direction = ["ArrowLeft", "ArrowUp"].includes(event.key) ? -1 : 1;
  } else if (["Home", "End"].includes(event.key) || (horizontalBar ? ["ArrowUp", "ArrowDown"] : ["ArrowLeft", "ArrowRight"]).includes(event.key)) {
    candidates = marks.filter((mark) => mark.dataset.seriesIndex === currentSeries);
    direction = ["ArrowLeft", "ArrowUp"].includes(event.key) ? -1 : 1;
  } else {
    candidates = marks.filter((mark) => currentCategory !== undefined
      ? mark.dataset.categoryIndex === currentCategory
      : mark.dataset.pointIndex === currentPoint);
    direction = ["ArrowLeft", "ArrowUp"].includes(event.key) ? -1 : 1;
  }
  if (!candidates.length) return;
  const currentIndex = Math.max(0, candidates.indexOf(current));
  const nextIndex = event.key === "Home"
    ? 0
    : event.key === "End"
      ? candidates.length - 1
      : (currentIndex + direction + candidates.length) % candidates.length;
  event.preventDefault();
  candidates[nextIndex]?.focus();
}

function pinFromPointer(event: ReactPointerEvent<SVGElement>): boolean {
  return event.pointerType !== "mouse";
}

type CartesianChartProps = {
  spec: NativeChartSpec;
  width: number;
  titleId: string;
  descriptionId: string;
} & ChartInteractionProps;

function BarGradientDefs({ prefix, seriesCount }: { prefix: string; seriesCount: number }) {
  return (
    <defs>
      {Array.from({ length: seriesCount }, (_, index) => {
        const color = COLORS[index % COLORS.length];
        return (
          <linearGradient key={index} id={`${prefix}-${index}`} x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stopColor={`color-mix(in srgb, ${color} 88%, var(--primary-text))`} />
            <stop offset="100%" stopColor={`color-mix(in srgb, ${color} 82%, var(--card-bg))`} />
          </linearGradient>
        );
      })}
    </defs>
  );
}

function HorizontalBarChart({
  spec,
  width,
  titleId,
  descriptionId,
  activeMark,
  hiddenEntries,
  pinned,
  onActivate,
  onClear,
  gradientPrefix,
}: CartesianChartProps & { gradientPrefix: string }) {
  const visibleSeries = spec.series
    .map((series, seriesIndex) => ({ series, seriesIndex }))
    .filter(({ seriesIndex }) => !hiddenEntries.has(seriesIndex));
  const categories = Array.from(new Set(spec.series.flatMap((series) => series.points.map((point) => String(point.x)))));
  const values = visibleSeries.flatMap(({ series }) => series.points.map((point) => point.y));
  const top = 18;
  const bottom = 38 + (spec.y_label ? 14 : 0);
  const height = Math.max(260, Math.min(420, top + bottom + categories.length * Math.max(24, visibleSeries.length * 18 + 8)));
  const maxCategoryCharacters = width < 400 ? 12 : 22;
  const categoryLines = categories.map((category) => splitCategoryLabel(category, maxCategoryCharacters));
  const widestCategory = Math.max(...categoryLines.flatMap((lines) => lines.map((line) => line.length)));
  const left = Math.min(width * 0.42, Math.max(spec.x_label ? 62 : 48, widestCategory * 6 + 14 + (spec.x_label ? 10 : 0)));
  const right = 18;
  const plotWidth = Math.max(100, width - left - right);
  const plotHeight = Math.max(120, height - top - bottom);
  const slot = plotHeight / Math.max(1, categories.length);
  const groupHeight = Math.min(slot * 0.72, 58);
  const barHeight = groupHeight / visibleSeries.length;
  const totalBars = visibleSeries.reduce((sum, { series }) => sum + series.points.length, 0);
  const showValueLabels = totalBars <= 8 && barHeight >= 18;
  const valueDomain = chartDomain(values, true, showValueLabels);
  const valueScale = scale(valueDomain.min, valueDomain.max, left, plotWidth);
  const formattedValueTicks = valueDomain.ticks.map((tick) => formatChartValue(tick, spec.y_format));
  const valueTickIndexes = chartNumericTickIndexes(valueDomain.ticks, formattedValueTicks, plotWidth);
  const zeroX = valueScale(0);
  const categoryY = (category: string) => top + ((categories.indexOf(category) + 0.5) / categories.length) * plotHeight;
  const maxCategoryTicks = Math.max(2, Math.floor(plotHeight / 24));
  const categoryStep = Math.max(1, Math.ceil(categories.length / maxCategoryTicks));
  const visibleCategoryIndexes = categories.map((_, index) => index).filter((index) => index % categoryStep === 0 || index === categories.length - 1);
  const categoryRows = (category: string): TooltipRow[] => visibleSeries.flatMap(({ series, seriesIndex }) => {
    const point = series.points.find((candidate) => String(candidate.x) === category);
    return point ? [{ entryIndex: seriesIndex, label: series.name, value: formatChartValue(point.y, spec.y_format), color: COLORS[seriesIndex % COLORS.length] }] : [];
  });

  return (
    <svg className="native-chart-svg" viewBox={`0 0 ${width} ${height}`} role="graphics-document" data-chart-type="bar" data-orientation="horizontal" aria-labelledby={`${titleId} ${descriptionId}`}>
      <desc id={descriptionId}>{chartDescription(spec)}</desc>
      <BarGradientDefs prefix={gradientPrefix} seriesCount={spec.series.length} />
      {valueTickIndexes.map((index) => {
        const tick = valueDomain.ticks[index];
        const x = valueScale(tick);
        const isZero = Math.abs(tick) < Number.EPSILON;
        return (
          <g key={`${tick}-${index}`}>
            <line className={`native-chart-grid${isZero ? " zero" : ""}`} x1={x} x2={x} y1={top} y2={top + plotHeight} />
            <text className="native-chart-tick native-chart-number" x={x} y={top + plotHeight + 20} textAnchor="middle">{formattedValueTicks[index]}</text>
          </g>
        );
      })}
      <line className="native-chart-axis" x1={left} x2={left + plotWidth} y1={top + plotHeight} y2={top + plotHeight} />
      <line className="native-chart-axis" x1={zeroX} x2={zeroX} y1={top} y2={top + plotHeight} />
      {visibleCategoryIndexes.map((categoryIndex) => {
        const category = categories[categoryIndex];
        const lines = categoryLines[categoryIndex];
        const y = categoryY(category);
        return (
          <text className={`native-chart-tick category${activeMark?.categoryIndex === categoryIndex ? " active" : ""}`} key={`${category}-${categoryIndex}`} x={left - 8} y={y - (lines.length - 1) * 5.5 + 4} textAnchor="end">
            {lines.map((line, lineIndex) => <tspan key={lineIndex} x={left - 8} dy={lineIndex === 0 ? 0 : 11}>{line}</tspan>)}
          </text>
        );
      })}
      <g className="native-chart-marks">
        {visibleSeries.flatMap(({ series, seriesIndex }, visibleIndex) => series.points.map((point, pointIndex) => {
          const category = String(point.x);
          const categoryIndex = categories.indexOf(category);
          const center = categoryY(category) - groupHeight / 2 + barHeight * (visibleIndex + 0.5);
          const valueX = valueScale(point.y);
          const x = Math.min(valueX, zeroX);
          const markWidth = Math.max(1, Math.abs(zeroX - valueX));
          const markHeight = Math.max(1, barHeight - 2);
          const formattedValue = formatChartValue(point.y, spec.y_format);
          const mark: ActiveMark = {
            entryIndex: seriesIndex,
            pointIndex,
            categoryIndex,
            svgX: valueX,
            svgY: center,
            left: (valueX / width) * 100,
            top: (center / height) * 100,
            heading: category,
            rows: categoryRows(category),
            placement: center < top + 48 ? "below" : "above",
          };
          const positive = point.y >= 0;
          const estimatedLabelWidth = formattedValue.length * 6;
          const outsideFits = positive
            ? valueX + 6 + estimatedLabelWidth <= left + plotWidth
            : valueX - 6 - estimatedLabelWidth >= left;
          const labelX = outsideFits ? valueX + (positive ? 6 : -6) : valueX + (positive ? -6 : 6);
          const labelAnchor = outsideFits ? (positive ? "start" : "end") : (positive ? "end" : "start");
          return (
            <g key={`${seriesIndex}-${pointIndex}`}>
              <path
                className={`${markClass(activeMark, seriesIndex, pointIndex, categoryIndex)} native-chart-bar-mark reveal-horizontal ${positive ? "origin-left" : "origin-right"}`}
                data-chart-mark="true"
                data-series-index={seriesIndex}
                data-point-index={pointIndex}
                data-category-index={categoryIndex}
                d={barMarkPath(x, center - barHeight / 2, markWidth, markHeight, 3, positive ? "right" : "left")}
                fill={`url(#${gradientPrefix}-${seriesIndex})`}
                stroke={COLORS[seriesIndex % COLORS.length]}
                strokeOpacity="0.24"
                style={{ animationDelay: `${(categoryIndex * visibleSeries.length + visibleIndex) * 22}ms` }}
                tabIndex={visibleIndex === 0 && pointIndex === 0 ? 0 : -1}
                role="graphics-symbol"
                aria-label={`${series.name}, ${category}: ${formattedValue}`}
                onPointerEnter={() => { if (!pinned) onActivate(mark); }}
                onPointerDown={(event) => onActivate(mark, pinFromPointer(event))}
                onPointerLeave={(event) => { if (event.pointerType === "mouse") onClear(); }}
                onFocus={() => onActivate(mark)}
                onBlur={onClear}
                onKeyDown={navigateChartMarks}
              />
              {showValueLabels ? <text className={`native-chart-value-label native-chart-number${outsideFits ? "" : " inside"}`} x={labelX} y={center + 3.5} textAnchor={labelAnchor}>{formattedValue}</text> : null}
            </g>
          );
        }))}
      </g>
      {spec.y_label ? <text className="native-chart-axis-label" x={left + plotWidth / 2} y={height - 6} textAnchor="middle">{spec.y_label}</text> : null}
      {spec.x_label ? <text className="native-chart-axis-label" x="13" y={top + plotHeight / 2} textAnchor="middle" transform={`rotate(-90 13 ${top + plotHeight / 2})`}>{spec.x_label}</text> : null}
    </svg>
  );
}

function CartesianChart({
  spec,
  width,
  titleId,
  descriptionId,
  activeMark,
  hiddenEntries,
  pinned,
  onActivate,
  onClear,
}: CartesianChartProps) {
  const gradientPrefix = useId().replace(/:/g, "");
  if (spec.type === "bar" && spec.orientation === "horizontal") {
    return <HorizontalBarChart spec={spec} width={width} titleId={titleId} descriptionId={descriptionId} activeMark={activeMark} hiddenEntries={hiddenEntries} pinned={pinned} onActivate={onActivate} onClear={onClear} gradientPrefix={gradientPrefix} />;
  }
  const visibleSeries = spec.series
    .map((series, seriesIndex) => ({ series, seriesIndex }))
    .filter(({ seriesIndex }) => !hiddenEntries.has(seriesIndex));
  const allPoints = visibleSeries.flatMap(({ series }) => series.points);
  const categories = Array.from(new Set(spec.series.flatMap((series) => series.points.map((point) => String(point.x)))));
  const numericX = spec.type === "scatter";
  const estimatedPlotWidth = Math.max(120, width - 88);
  const totalBars = visibleSeries.reduce((sum, { series }) => sum + series.points.length, 0);
  const estimatedBarWidth = estimatedPlotWidth * 0.72 / Math.max(1, categories.length * visibleSeries.length);
  const showValueLabels = spec.type === "bar" && totalBars <= 8 && estimatedBarWidth >= 34;
  const yDomain = chartDomain(allPoints.map((point) => point.y), spec.type === "bar", showValueLabels);
  const formattedYTicks = yDomain.ticks.map((tick) => formatChartValue(tick, spec.y_format));
  const widestYTick = Math.max(...formattedYTicks.map((tick) => tick.length));
  const left = Math.min(94, Math.max(spec.y_label ? 62 : 48, widestYTick * 6 + 16 + (spec.y_label ? 10 : 0)));
  const right = 18;
  const top = showValueLabels ? 24 : 18;
  const wrappedCategories = categories.some((category) => category.length > 12);
  const bottom = 44 + (wrappedCategories ? 12 : 0) + (spec.x_label ? 14 : 0);
  const height = Math.max(260, Math.min(340, Math.round(width * 0.52)));
  const plotWidth = Math.max(80, width - left - right);
  const plotHeight = Math.max(120, height - top - bottom);
  const xDomain = numericX ? chartDomain(allPoints.map((point) => Number(point.x)), false) : null;
  const xScale = scale(xDomain?.min ?? 0, xDomain?.max ?? Math.max(1, categories.length - 1), left, plotWidth);
  const formattedXTicks = xDomain?.ticks.map((tick) => formatChartValue(tick, spec.x_format)) ?? [];
  const xTickIndexes = xDomain ? chartNumericTickIndexes(xDomain.ticks, formattedXTicks, plotWidth) : [];
  const yScale = scale(yDomain.min, yDomain.max, top + plotHeight, -plotHeight);
  const zeroY = yDomain.min <= 0 && yDomain.max >= 0 ? yScale(0) : top + plotHeight;
  const categoryX = (value: string) => {
    const index = categories.indexOf(value);
    if (spec.type === "bar") return left + ((index + 0.5) / categories.length) * plotWidth;
    return categories.length === 1 ? left + plotWidth / 2 : xScale(index);
  };
  const categoryIndexes = chartCategoryTickIndexes(categories, plotWidth);
  const maxCategoryCharacters = Math.max(6, Math.min(16, Math.floor(plotWidth / Math.max(2, categoryIndexes.length) / 5.8)));
  const categoryRows = (category: string): TooltipRow[] => visibleSeries.flatMap(({ series, seriesIndex }) => {
    const point = series.points.find((candidate) => String(candidate.x) === category);
    return point ? [{ entryIndex: seriesIndex, label: series.name, value: formatChartValue(point.y, spec.y_format), color: COLORS[seriesIndex % COLORS.length] }] : [];
  });

  return (
    <svg className="native-chart-svg" viewBox={`0 0 ${width} ${height}`} role="graphics-document" data-chart-type={spec.type} data-orientation={spec.type === "bar" ? "vertical" : undefined} aria-labelledby={`${titleId} ${descriptionId}`}>
      <desc id={descriptionId}>{chartDescription(spec)}</desc>
      {spec.type === "bar" ? <BarGradientDefs prefix={gradientPrefix} seriesCount={spec.series.length} /> : null}
      {yDomain.ticks.map((tick, index) => {
        const y = yScale(tick);
        const isZero = Math.abs(tick) < Number.EPSILON;
        return (
          <g key={`${tick}-${index}`}>
            <line className={`native-chart-grid${isZero ? " zero" : ""}`} x1={left} x2={left + plotWidth} y1={y} y2={y} />
            <text className="native-chart-tick native-chart-number" x={left - 8} y={y + 4} textAnchor="end">{formattedYTicks[index]}</text>
          </g>
        );
      })}
      <line className="native-chart-axis" x1={left} x2={left} y1={top} y2={top + plotHeight} />
      <line className="native-chart-axis" x1={left} x2={left + plotWidth} y1={zeroY} y2={zeroY} />
      {numericX
        ? xTickIndexes.map((index) => (
            <text className="native-chart-tick native-chart-number" key={xDomain?.ticks[index]} x={xScale(xDomain?.ticks[index] ?? 0)} y={top + plotHeight + 20} textAnchor="middle">{formattedXTicks[index]}</text>
          ))
        : categoryIndexes.map((categoryIndex) => {
            const category = categories[categoryIndex];
            const lines = splitCategoryLabel(category, maxCategoryCharacters);
            return (
              <text className={`native-chart-tick category${activeMark?.categoryIndex === categoryIndex ? " active" : ""}`} key={`${category}-${categoryIndex}`} x={categoryX(category)} y={top + plotHeight + 19} textAnchor="middle">
                {lines.map((line, lineIndex) => <tspan key={lineIndex} x={categoryX(category)} dy={lineIndex === 0 ? 0 : 11}>{line}</tspan>)}
              </text>
            );
          })}
      {activeMark && (spec.type === "line" || spec.type === "scatter") ? (
        <g className="native-chart-crosshair" aria-hidden="true">
          <line x1={activeMark.svgX} x2={activeMark.svgX} y1={top} y2={top + plotHeight} />
          {spec.type === "scatter" ? <line x1={left} x2={left + plotWidth} y1={activeMark.svgY} y2={activeMark.svgY} /> : null}
        </g>
      ) : null}
      <g className="native-chart-marks">
        {spec.type === "bar" ? visibleSeries.flatMap(({ series, seriesIndex }, visibleIndex) => {
          const slot = plotWidth / Math.max(categories.length, 1);
          const groupWidth = Math.min(slot * 0.72, 72);
          const barWidth = groupWidth / visibleSeries.length;
          return series.points.map((point, pointIndex) => {
            const category = String(point.x);
            const categoryIndex = categories.indexOf(category);
            const x = categoryX(category) - groupWidth / 2 + barWidth * visibleIndex;
            const center = x + barWidth / 2;
            const valueY = yScale(point.y);
            const y = Math.min(valueY, zeroY);
            const barHeight = Math.max(1, Math.abs(zeroY - valueY));
            const markWidth = Math.max(1, barWidth - 2);
            const color = COLORS[seriesIndex % COLORS.length];
            const mark: ActiveMark = {
              entryIndex: seriesIndex,
              pointIndex,
              categoryIndex,
              svgX: center,
              svgY: valueY,
              left: (center / width) * 100,
              top: (valueY / height) * 100,
              heading: category,
              rows: categoryRows(category),
              placement: valueY < top + 48 ? "below" : "above",
            };
            const labelY = Math.max(top + 10, Math.min(top + plotHeight - 4, point.y < 0 ? y + barHeight + 13 : y - 7));
            return (
              <g key={`${seriesIndex}-${pointIndex}`}>
                <path
                  className={`${markClass(activeMark, seriesIndex, pointIndex, categoryIndex)} native-chart-bar-mark reveal-vertical ${point.y < 0 ? "origin-top" : "origin-bottom"}`}
                  data-chart-mark="true"
                  data-series-index={seriesIndex}
                  data-point-index={pointIndex}
                  data-category-index={categoryIndex}
                  d={barMarkPath(x, y, markWidth, barHeight, 3, point.y < 0 ? "bottom" : "top")}
                  fill={`url(#${gradientPrefix}-${seriesIndex})`}
                  stroke={color}
                  strokeOpacity="0.24"
                  style={{ animationDelay: `${(categoryIndex * visibleSeries.length + visibleIndex) * 22}ms` }}
                  tabIndex={visibleIndex === 0 && pointIndex === 0 ? 0 : -1}
                  role="graphics-symbol"
                  aria-label={`${series.name}, ${category}: ${formatChartValue(point.y, spec.y_format)}`}
                  onPointerEnter={() => { if (!pinned) onActivate(mark); }}
                  onPointerDown={(event) => onActivate(mark, pinFromPointer(event))}
                  onPointerLeave={(event) => { if (event.pointerType === "mouse") onClear(); }}
                  onFocus={() => onActivate(mark)}
                  onBlur={onClear}
                  onKeyDown={navigateChartMarks}
                />
                {showValueLabels ? <text className="native-chart-value-label native-chart-number" x={center} y={labelY} textAnchor="middle">{formatChartValue(point.y, spec.y_format)}</text> : null}
              </g>
            );
          });
        }) : visibleSeries.map(({ series, seriesIndex }, visibleIndex) => {
          const points = series.points.map((point) => `${numericX ? xScale(Number(point.x)) : categoryX(String(point.x))},${yScale(point.y)}`).join(" ");
          const color = COLORS[seriesIndex % COLORS.length];
          return (
            <g key={seriesIndex}>
              {spec.type === "line" ? (
                <polyline
                  className={`native-chart-series style-${seriesIndex % 3}${activeMark && activeMark.entryIndex !== seriesIndex ? " dimmed" : ""}`}
                  points={points}
                  fill="none"
                  stroke={color}
                  strokeWidth="2.25"
                  strokeDasharray={seriesIndex % 3 === 1 ? "7 4" : seriesIndex % 3 === 2 ? "2 4" : undefined}
                />
              ) : null}
              {series.points.map((point, pointIndex) => {
                const category = String(point.x);
                const categoryIndex = numericX ? undefined : categories.indexOf(category);
                const cx = numericX ? xScale(Number(point.x)) : categoryX(category);
                const cy = yScale(point.y);
                const mark: ActiveMark = {
                  entryIndex: seriesIndex,
                  pointIndex,
                  categoryIndex,
                  svgX: cx,
                  svgY: cy,
                  left: (cx / width) * 100,
                  top: (cy / height) * 100,
                  heading: numericX ? formatChartValue(Number(point.x), spec.x_format) : category,
                  rows: numericX
                    ? [{ entryIndex: seriesIndex, label: series.name, value: formatChartValue(point.y, spec.y_format), color }]
                    : categoryRows(category),
                  placement: cy < top + 48 ? "below" : "above",
                };
                return (
                  <g
                    key={pointIndex}
                    className={markClass(activeMark, seriesIndex, pointIndex, categoryIndex)}
                    data-chart-mark="true"
                    data-series-index={seriesIndex}
                    data-point-index={pointIndex}
                    data-category-index={categoryIndex}
                    tabIndex={visibleIndex === 0 && pointIndex === 0 ? 0 : -1}
                    role="graphics-symbol"
                    aria-label={`${series.name}, ${numericX ? formatChartValue(Number(point.x), spec.x_format) : category}: ${formatChartValue(point.y, spec.y_format)}`}
                    onPointerEnter={() => { if (!pinned) onActivate(mark); }}
                    onPointerDown={(event) => onActivate(mark, pinFromPointer(event))}
                    onPointerLeave={(event) => { if (event.pointerType === "mouse") onClear(); }}
                    onFocus={() => onActivate(mark)}
                    onBlur={onClear}
                    onKeyDown={navigateChartMarks}
                  >
                    <circle className="native-chart-hit" cx={cx} cy={cy} r="12" />
                    <circle className="native-chart-point" cx={cx} cy={cy} r={spec.type === "scatter" ? 4.5 : 3.5} fill={spec.type === "line" ? "var(--card-bg)" : color} stroke={color} strokeWidth="2" />
                  </g>
                );
              })}
            </g>
          );
        })}
      </g>
      {spec.x_label ? <text className="native-chart-axis-label" x={left + plotWidth / 2} y={height - 6} textAnchor="middle">{spec.x_label}</text> : null}
      {spec.y_label ? <text className="native-chart-axis-label" x="13" y={top + plotHeight / 2} textAnchor="middle" transform={`rotate(-90 13 ${top + plotHeight / 2})`}>{spec.y_label}</text> : null}
    </svg>
  );
}

function polarPoint(cx: number, cy: number, radius: number, angle: number): [number, number] {
  const radians = ((angle - 90) * Math.PI) / 180;
  return [cx + radius * Math.cos(radians), cy + radius * Math.sin(radians)];
}

function piePath(cx: number, cy: number, radius: number, start: number, end: number): string {
  if (end - start >= 359.999) return `M ${cx} ${cy - radius} A ${radius} ${radius} 0 1 1 ${cx - 0.01} ${cy - radius} Z`;
  const [startX, startY] = polarPoint(cx, cy, radius, end);
  const [endX, endY] = polarPoint(cx, cy, radius, start);
  return `M ${cx} ${cy} L ${startX} ${startY} A ${radius} ${radius} 0 ${end - start > 180 ? 1 : 0} 0 ${endX} ${endY} Z`;
}

function PieChart({
  spec,
  width,
  titleId,
  descriptionId,
  activeMark,
  hiddenEntries,
  pinned,
  onActivate,
  onClear,
}: {
  spec: NativeChartSpec;
  width: number;
  titleId: string;
  descriptionId: string;
} & ChartInteractionProps) {
  const height = Math.max(240, Math.min(320, Math.round(width * 0.46)));
  const cx = width / 2;
  const cy = height / 2;
  const radius = Math.min(116, Math.min(width, height) * 0.36);
  const points = spec.series[0].points
    .map((point, pointIndex) => ({ point, pointIndex }))
    .filter(({ pointIndex }) => !hiddenEntries.has(pointIndex));
  const total = points.reduce((sum, { point }) => sum + point.y, 0);
  let angle = 0;
  return (
    <svg className="native-chart-svg native-chart-pie" viewBox={`0 0 ${width} ${height}`} role="graphics-document" data-chart-type="pie" aria-labelledby={`${titleId} ${descriptionId}`}>
      <desc id={descriptionId}>{chartDescription(spec)}</desc>
      <g className="native-chart-marks">
        {points.map(({ point, pointIndex }, visibleIndex) => {
          const start = angle;
          const end = angle + (point.y / total) * 360;
          angle = end;
          const color = COLORS[pointIndex % COLORS.length];
          const [svgX, svgY] = polarPoint(cx, cy, radius * 0.66, (start + end) / 2);
          const share = (point.y / total) * 100;
          const mark: ActiveMark = {
            entryIndex: pointIndex,
            pointIndex,
            categoryIndex: pointIndex,
            svgX,
            svgY,
            left: (svgX / width) * 100,
            top: (svgY / height) * 100,
            heading: String(point.x),
            rows: [{ entryIndex: pointIndex, label: spec.series[0].name, value: `${formatChartValue(point.y, spec.y_format)} · ${formatChartValue(share, { style: "percent", precision: 1 })}`, color }],
            placement: svgY < cy - radius * 0.55 ? "below" : "above",
          };
          return (
            <path
              key={pointIndex}
              className={markClass(activeMark, pointIndex, pointIndex, pointIndex)}
              data-chart-mark="true"
              data-series-index="0"
              data-point-index={pointIndex}
              data-category-index={pointIndex}
              d={piePath(cx, cy, radius, start, end)}
              fill={color}
              stroke="var(--card-bg)"
              strokeWidth="2"
              tabIndex={visibleIndex === 0 ? 0 : -1}
              role="graphics-symbol"
              aria-label={`${point.x}: ${formatChartValue(point.y, spec.y_format)}, ${formatChartValue(share, { style: "percent", precision: 1 })}`}
              onPointerEnter={() => { if (!pinned) onActivate(mark); }}
              onPointerDown={(event) => onActivate(mark, pinFromPointer(event))}
              onPointerLeave={(event) => { if (event.pointerType === "mouse") onClear(); }}
              onFocus={() => onActivate(mark)}
              onBlur={onClear}
              onKeyDown={navigateChartMarks}
            />
          );
        })}
      </g>
    </svg>
  );
}

function ChartLegend({
  spec,
  hiddenEntries,
  onToggle,
}: {
  spec: NativeChartSpec;
  hiddenEntries: ReadonlySet<number>;
  onToggle: (index: number) => void;
}) {
  const visiblePieTotal = spec.type === "pie"
    ? spec.series[0].points.reduce((sum, point, index) => sum + (hiddenEntries.has(index) ? 0 : point.y), 0)
    : 0;
  const entries = spec.type === "pie"
    ? spec.series[0].points.map((point, index) => ({
        name: String(point.x),
        color: COLORS[index % COLORS.length],
        value: point.y,
        detail: hiddenEntries.has(index)
          ? formatChartValue(point.y, spec.y_format)
          : `${formatChartValue(point.y, spec.y_format)} · ${formatChartValue((point.y / visiblePieTotal) * 100, { style: "percent", precision: 1 })}`,
      }))
    : spec.series.map((series, index) => ({ name: series.name, color: COLORS[index % COLORS.length], value: 1, detail: undefined }));
  if (entries.length < 2 && spec.type !== "pie") return null;
  const visibleCount = entries.length - hiddenEntries.size;
  return (
    <div className={`native-chart-legend ${spec.type}`} role="group" aria-label="Chart legend">
      {entries.map((entry, index) => (
        <button
          key={index}
          type="button"
          className={hiddenEntries.has(index) ? "hidden" : undefined}
          aria-pressed={!hiddenEntries.has(index)}
          aria-label={`${hiddenEntries.has(index) ? "Show" : "Hide"} ${entry.name}`}
          disabled={!hiddenEntries.has(index) && (visibleCount === 1 || (spec.type === "pie" && entries.filter((_, otherIndex) => otherIndex !== index && !hiddenEntries.has(otherIndex)).reduce((sum, other) => sum + other.value, 0) <= 0))}
          onClick={() => onToggle(index)}
        >
          <i className={spec.type === "line" ? `style-${index % 3}` : undefined} style={{ background: entry.color, borderColor: entry.color }} aria-hidden="true" />
          <span>{entry.name}{entry.detail ? <small>{entry.detail}</small> : null}</span>
        </button>
      ))}
    </div>
  );
}

function ChartTooltip({ mark }: { mark: ActiveMark }) {
  const edge = mark.left < 20 ? " edge-left" : mark.left > 80 ? " edge-right" : "";
  return (
    <div
      className={`native-chart-tooltip ${mark.placement}${edge}`}
      data-testid="native-chart-tooltip"
      role="tooltip"
      style={{ left: `clamp(82px, ${mark.left}%, calc(100% - 82px))`, top: `${mark.top}%` }}
    >
      <strong>{mark.heading}</strong>
      <span>
        {mark.rows.map((row) => (
          <small key={row.entryIndex} className={row.entryIndex === mark.entryIndex ? "active" : undefined}>
            <i style={{ background: row.color }} aria-hidden="true" />
            <em>{row.label}</em>
            <b>{row.value}</b>
          </small>
        ))}
      </span>
    </div>
  );
}

function AccessibleDataTable({ spec }: { spec: NativeChartSpec }) {
  const pieTotal = spec.type === "pie" ? spec.series[0].points.reduce((sum, point) => sum + point.y, 0) : 0;
  return (
    <table className="native-chart-data">
      <caption>{spec.title} data</caption>
      <thead><tr><th>Series</th><th>X</th><th>Y</th>{spec.type === "pie" ? <th>Share</th> : null}</tr></thead>
      <tbody>{spec.series.flatMap((series, seriesIndex) => series.points.map((point, pointIndex) => (
        <tr key={`${seriesIndex}-${pointIndex}`}>
          <td>{series.name}</td>
          <td>{typeof point.x === "number" ? formatChartValue(point.x, spec.x_format) : point.x}</td>
          <td>{formatChartValue(point.y, spec.y_format)}</td>
          {spec.type === "pie" ? <td>{formatChartValue((point.y / pieTotal) * 100, { style: "percent", precision: 1 })}</td> : null}
        </tr>
      )))}</tbody>
    </table>
  );
}

export function NativeChartView({ argumentsText, result, status = "done" }: NativeChartViewProps) {
  const titleId = useId();
  const descriptionId = useId();
  const interactionId = useId();
  const plotRef = useRef<HTMLDivElement>(null);
  const spec = useMemo(() => parseNativeChartSpec(result) ?? specFromArguments(argumentsText), [argumentsText, result]);
  const [activeMark, setActiveMark] = useState<ActiveMark | null>(null);
  const [hiddenEntries, setHiddenEntries] = useState<Set<number>>(() => new Set());
  const [pinned, setPinned] = useState(false);
  const [plotWidth, setPlotWidth] = useState(640);
  useEffect(() => {
    setActiveMark(null);
    setHiddenEntries(new Set());
    setPinned(false);
  }, [spec]);
  useEffect(() => {
    const element = plotRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const update = () => setPlotWidth(Math.max(280, Math.min(680, Math.round(element.getBoundingClientRect().width))));
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [spec]);
  if (status === "error") return <div className="native-chart-state" role="alert">Chart rendering failed.</div>;
  if (!spec) return <div className="native-chart-state" role="status">{status === "running" ? "Rendering chart..." : "Chart data is unavailable."}</div>;
  const entryCount = spec.type === "pie" ? spec.series[0].points.length : spec.series.length;
  const clearPinned = () => {
    setPinned(false);
    setActiveMark(null);
  };
  const toggleEntry = (index: number) => {
    clearPinned();
    setHiddenEntries((current) => {
      const next = new Set(current);
      if (next.has(index)) {
        next.delete(index);
      } else {
        if (entryCount - next.size <= 1) return current;
        if (spec.type === "pie") {
          const remainingTotal = spec.series[0].points.reduce(
            (sum, point, pointIndex) => sum + (pointIndex !== index && !next.has(pointIndex) ? point.y : 0),
            0,
          );
          if (remainingTotal <= 0) return current;
        }
        next.add(index);
      }
      return next;
    });
  };
  const interactions: ChartInteractionProps = {
    activeMark,
    hiddenEntries,
    pinned,
    onActivate: (mark, pin = false) => {
      setActiveMark(mark);
      if (pin) setPinned(true);
    },
    onClear: () => { if (!pinned) setActiveMark(null); },
  };
  return (
    <section
      className="native-chart-card"
      data-testid="native-chart-view"
      aria-label={`${spec.title} chart`}
      aria-describedby={interactionId}
      onPointerDown={(event) => {
        if (pinned && event.target instanceof Element && !event.target.closest("[data-chart-mark]")) clearPinned();
      }}
    >
      <header className="native-chart-header">
        <div><span>Chart</span><h3 id={titleId}>{spec.title}</h3>{spec.subtitle ? <p>{spec.subtitle}</p> : null}</div>
        <strong>{spec.type === "bar" && spec.orientation === "horizontal" ? "horizontal bar" : spec.type}</strong>
      </header>
      <div className="native-chart-body">
        <p id={interactionId} className="native-chart-sr-only">Focus a data mark. {spec.type === "bar" && spec.orientation === "horizontal" ? "Use up and down to move within a series and left and right to move between series." : "Use left and right to move within a series and up and down to move between series."} Use the legend buttons to show or hide data.</p>
        <div className="native-chart-plot" ref={plotRef}>
          {spec.type === "pie"
            ? <PieChart spec={spec} width={plotWidth} titleId={titleId} descriptionId={descriptionId} {...interactions} />
            : <CartesianChart spec={spec} width={plotWidth} titleId={titleId} descriptionId={descriptionId} {...interactions} />}
          {activeMark ? <ChartTooltip mark={activeMark} /> : null}
        </div>
        <ChartLegend spec={spec} hiddenEntries={hiddenEntries} onToggle={toggleEntry} />
        <AccessibleDataTable spec={spec} />
      </div>
    </section>
  );
}
