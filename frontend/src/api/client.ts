import type { HealthResponse } from "./types/HealthResponse";

/** Base URL for all API calls.  Empty string = same origin (works in both dev
 *  with Vite proxy and in production served by foreman-server). */
const BASE = "";

class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    throw new ApiError(res.status, `GET ${path} → ${res.status}`);
  }
  return res.json() as Promise<T>;
}

export const api = {
  health: () => get<HealthResponse>("/api/health"),
};

export { ApiError };
