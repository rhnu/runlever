import { dateTimeAttribute, formatDateTime, statuses } from "./model";
import type { Status } from "./model";

const paths = {
  play: "m6 3 12 9-12 9z",
  terminal: "m4 7 5 5-5 5m8 0h7",
  "chevron-right": "m9 5 7 7-7 7",
  refresh: "M20 4v5h-5M4 20v-5h5m-4.6-5a8 8 0 0 1 13.2-4L20 9M4 15l2.4 3A8 8 0 0 0 19.6 14",
  x: "m6 6 12 12M18 6 6 18",
  "arrow-up-right": "M6 18 18 6M6 6h12v12",
} as const;

export function Icon({ name }: { name: keyof typeof paths }) {
  return (
    <svg
      className="icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={paths[name]} />
    </svg>
  );
}

export function StatusBadge({ status }: { status: Status }) {
  return <span className={`badge badge-${status}`}>{statuses[status]}</span>;
}

export function RunTime({ timestamp }: { timestamp: number }) {
  return <time dateTime={dateTimeAttribute(timestamp)}>{formatDateTime(timestamp)}</time>;
}

export function Feedback({ children, retry }: { children: string; retry?: () => void }) {
  return (
    <div className={`feedback${retry ? " feedback-error" : ""}`} role={retry ? "alert" : "status"}>
      <span>{children}</span>
      {retry && (
        <button className="button button-secondary" onClick={retry}>
          重试
        </button>
      )}
    </div>
  );
}
