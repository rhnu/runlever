export interface ActionInput {
  id: string;
  label: string;
  options: { id: string; label: string }[];
}

export interface Action {
  id: string;
  label: string;
  group?: { id: string; label: string };
  inputs: ActionInput[];
}

export const statuses = {
  queued: "排队中",
  running: "运行中",
  succeeded: "成功",
  failed: "失败",
  interrupted: "已中断",
  queue_timeout: "排队超时",
} as const;

export const sources = { web: "网页", webhook: "Webhook" } as const;

export type Status = keyof typeof statuses;
export type Source = keyof typeof sources;
export type Selections = Record<string, string>;

export interface Run {
  id: number;
  action: string;
  label: string;
  target: string;
  selections: Selections;
  source: Source;
  status: Status;
  submitted_at: number;
  started_at: number | null;
  finished_at: number | null;
  exit_code: number | null;
  error: string | null;
  logs_expired: boolean;
}

export interface LogChunk {
  seq: number;
  stream: "stdout" | "stderr";
  text: string;
  at: number;
}

export interface Filters {
  action: string;
  source: Source | "";
  status: Status | "";
}

const dateTime = new Intl.DateTimeFormat("zh-CN", {
  dateStyle: "medium",
  timeStyle: "medium",
});
const time = new Intl.DateTimeFormat("zh-CN", {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hourCycle: "h23",
});

export const formatTime = (seconds: number) => time.format(seconds * 1000);
export const formatDateTime = (seconds: number) => dateTime.format(seconds * 1000);
export const dateTimeAttribute = (seconds: number) => new Date(seconds * 1000).toISOString();

export function selectionLabel(
  action: Action | undefined,
  key: string,
  value: string,
  target?: Action,
) {
  const input =
    action?.inputs.find((field) => field.id === key) ??
    target?.inputs.find((field) => field.id === key);
  return {
    label: input?.label ?? key,
    value: input?.options.find((option) => option.id === value)?.label ?? value,
  };
}

export function formatDuration(startedAt: number, finishedAt: number) {
  const seconds = Math.max(0, finishedAt - startedAt);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return [hours ? `${hours} 小时` : "", minutes ? `${minutes} 分` : "", `${remainder} 秒`]
    .filter(Boolean)
    .join(" ");
}

export function matchesFilters(run: Run, filters: Filters) {
  return (
    (!filters.action || filters.action === run.action) &&
    (!filters.status || filters.status === run.status) &&
    (!filters.source || filters.source === run.source)
  );
}
