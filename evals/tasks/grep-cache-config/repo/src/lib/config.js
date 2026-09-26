const env = globalThis.process?.env ?? {};

export const config = {
  port: Number(env.PORT ?? 8080),
  defaultQuota: Number(env.DEFAULT_QUOTA ?? 1000),
  cacheTtlSeconds: Number(env.CACHE_TTL_SECONDS ?? 60),
  logLevel: env.LOG_LEVEL ?? "info",
};
