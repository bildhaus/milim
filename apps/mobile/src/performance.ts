const MOBILE_PERF_PREFIX = 'milim.mobile';

function performanceApi(): Performance | null {
  return typeof performance === 'undefined' ? null : performance;
}

export function mobilePerfMark(name: string): void {
  try {
    performanceApi()?.mark(`${MOBILE_PERF_PREFIX}.${name}`);
  } catch {
    // Performance marks are diagnostics only and must never affect the app.
  }
}

export function mobilePerfMeasure(
  name: string,
  start: string,
  end: string,
): void {
  try {
    const api = performanceApi();
    if (!api) return;
    const measureName = `${MOBILE_PERF_PREFIX}.${name}`;
    const startName = `${MOBILE_PERF_PREFIX}.${start}`;
    const endName = `${MOBILE_PERF_PREFIX}.${end}`;
    api.clearMeasures(measureName);
    api.measure(measureName, startName, endName);
    api.clearMarks(startName);
    api.clearMarks(endName);
  } catch {
    // A missing mark can occur during Fast Refresh or lifecycle interruption.
  }
}

export function mobileStartupTiming(): unknown {
  return (performanceApi() as (Performance & {rnStartupTiming?: unknown}) | null)
    ?.rnStartupTiming;
}
