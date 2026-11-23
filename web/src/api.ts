import type { Action, Filters, Run, Selections } from "./model";

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly detail: { error?: string; run_id?: number },
  ) {
    super(detail.error || `HTTP ${status}`);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, { ...init, cache: "no-store" });
  if (!response.ok) {
    const detail = await response.json().catch(() => null);
    throw new ApiError(response.status, detail ?? {});
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

function post<T>(path: string, body?: object) {
  return request<T>(path, {
    method: "POST",
    ...(body && {
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  });
}

export const api = {
  login: (token: string) => post<void>("/login", { token }),
  logout: () => post<void>("/logout"),
  actions: (signal: AbortSignal) => request<Action[]>("/actions", { signal }),
  start: (id: string, selections: Selections) =>
    post<{ run_id: number }>(`/actions/${encodeURIComponent(id)}/runs`, selections),
  run: (id: number, signal: AbortSignal) => request<Run>(`/runs/${id}`, { signal }),
  runs: (filters: Filters, before: number | undefined, signal: AbortSignal) => {
    const params = new URLSearchParams();
    if (filters.action) params.set("action", filters.action);
    if (filters.status) params.set("status", filters.status);
    if (filters.source) params.set("source", filters.source);
    if (before !== undefined) params.set("before", String(before));
    return request<Run[]>(`/runs?${params}`, { signal });
  },
  events: (id: number, after: number | undefined) =>
    new EventSource(`/api/runs/${id}/events${after === undefined ? "" : `?after=${after}`}`),
};

export function isUnauthorized(error: unknown) {
  return error instanceof ApiError && error.status === 401;
}
