import type { MilimUsageSummary } from "../lib/usageMetrics";

export function MilimUsageRidgeline({ usage }: { usage: MilimUsageSummary }) {
  if (!usage.hasUsage) return null;
  const months = usage.months.slice(-3);
  const width = 440;
  const gradientId = "usage-ridge-fill-gradient";
  const amplitude = 40;
  const lineSpacing = 17;
  const lineWidth = 1.35;
  const topPad = amplitude + 8;
  const height = topPad + lineSpacing * (months.length - 1) + 10;
  const maxValue = Math.max(1, ...months.flatMap((month) => month.days));
  const ridges = months.map((month, index) => {
    const base = topPad + index * lineSpacing;
    const line = ridgePath(month.days, base, amplitude, width, maxValue);
    const empty = month.days.every((value) => value === 0);
    const depth =
      months.length <= 1 ? 1 : index / Math.max(1, months.length - 1);
    return {
      base,
      closed: `${line} L${width},${base} L0,${base} Z`,
      depth,
      empty,
      fillOpacity: empty ? 0 : 0.18 + depth * 0.18,
      index,
      key: month.key,
      line,
      lineOpacity: empty ? 0.16 : 0.5 + depth * 0.22,
    };
  });
  const activeRidges = ridges.filter((ridge) => !ridge.empty);
  const drawRidges = [
    ...ridges.filter((ridge) => ridge.empty),
    ...ridges.filter((ridge) => !ridge.empty),
  ];
  const blockersForRidge = (ridge: (typeof ridges)[number]) =>
    ridge.empty
      ? activeRidges
      : activeRidges.filter((blocker) => blocker.index > ridge.index);
  const maskIdForRidge = (ridge: (typeof ridges)[number]) =>
    `usage-ridge-occlusion-${ridge.index}`;

  return (
    <section
      className="usage-empty-panel"
      data-testid="empty-usage-ridgeline"
      aria-label="Milim usage"
    >
      <svg
        className="usage-ridgeline"
        role="img"
        aria-label="Monthly ridgeline chart of local thread activity"
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
      >
        <defs>
          <linearGradient id={gradientId} x1="0" x2="0" y1="0" y2="1">
            <stop
              offset="0%"
              stopColor="var(--accent-light)"
              stopOpacity="0.64"
            />
            <stop offset="54%" stopColor="var(--accent)" stopOpacity="0.28" />
            <stop
              offset="100%"
              stopColor="var(--panel-bg)"
              stopOpacity="0"
            />
          </linearGradient>
          {ridges.map((ridge) => {
            const blockers = blockersForRidge(ridge);
            if (!blockers.length) return null;
            return (
              <mask
                id={maskIdForRidge(ridge)}
                key={ridge.key}
                maskUnits="userSpaceOnUse"
              >
                <rect width={width} height={height} fill="white" />
                {blockers.map((blocker) => (
                  <path key={blocker.key} d={blocker.closed} fill="black" />
                ))}
              </mask>
            );
          })}
        </defs>
        {drawRidges.map((ridge) => {
          const blockers = blockersForRidge(ridge);
          return (
            <g
              key={ridge.key}
              className="usage-ridge-row"
              mask={
                blockers.length ? `url(#${maskIdForRidge(ridge)})` : undefined
              }
            >
              <path
                className="usage-ridge-fill"
                d={ridge.closed}
                style={{ opacity: ridge.fillOpacity }}
                data-empty={ridge.empty || undefined}
              />
              <path
                className="usage-ridge-line"
                d={ridge.line}
                style={{ opacity: ridge.lineOpacity }}
                strokeWidth={lineWidth}
                data-empty={ridge.empty || undefined}
              />
            </g>
          );
        })}
      </svg>
      <div className="usage-empty-footer">
        <div className="usage-empty-metrics">
          {usage.metrics.map((metric) => (
            <div className="usage-empty-metric" key={metric.label}>
              <span className="usage-empty-metric-value">{metric.value}</span>
              <span className="usage-empty-metric-label">{metric.label}</span>
            </div>
          ))}
        </div>
        <div className="usage-empty-latest">
          {usage.months[usage.months.length - 1]?.label}
        </div>
      </div>
    </section>
  );
}

function ridgePath(
  values: number[],
  base: number,
  amplitude: number,
  width: number,
  maxValue: number,
): string {
  const points = values.map((value, index) => {
    const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * width;
    const y = base - (value / maxValue) * amplitude;
    return [x, y];
  });
  let line = `M${points[0][0]},${points[0][1]}`;
  for (let index = 1; index < points.length; index += 1) {
    const [x0, y0] = points[index - 1];
    const [x1, y1] = points[index];
    const midX = (x0 + x1) / 2;
    line += ` C${midX},${y0} ${midX},${y1} ${x1},${y1}`;
  }
  return line;
}
