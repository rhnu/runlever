import { useLayoutEffect, useRef, useState } from "react";
import {
  formatDateTime,
  formatDuration,
  formatTime,
  dateTimeAttribute,
  selectionLabel,
  sources,
  statuses,
} from "./model";
import type { Action, Run, Status } from "./model";
import { historyHref, Link } from "./route";
import { useRunStream } from "./use-run-stream";

const linesPerPage = 400;

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

interface RunPanelProps {
  id: number;
  mode: "actions" | "history";
  actions: Action[];
  onUnauthorized: () => void;
  onRun?: (run: Run) => void;
}

export function RunPanel({ id, mode, actions, onUnauthorized, onRun }: RunPanelProps) {
  const { run, buffer, connection, error, revision, retry } = useRunStream(
    id,
    onUnauthorized,
    onRun,
  );
  const [follow, setFollow] = useState(true);
  const [selectedPage, setSelectedPage] = useState(0);
  const [seenLines, setSeenLines] = useState(0);
  const viewport = useRef<HTMLElement>(null);
  const lastPage = Math.max(0, Math.ceil(buffer.lines.length / linesPerPage) - 1);
  const page = follow ? lastPage : Math.min(lastPage, selectedPage);
  const lines = buffer.lines.slice(page * linesPerPage, (page + 1) * linesPerPage);
  const newLines = follow ? 0 : buffer.lines.length - seenLines;
  const hasLogs = Boolean(run && run.started_at !== null && !run.logs_expired);

  useLayoutEffect(() => {
    if (follow && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  }, [follow, revision, hasLogs]);

  function toggleFollow() {
    if (follow) pauseFollow();
    else setFollow(true);
  }

  function pauseFollow() {
    if (!follow) return;
    setSelectedPage(page);
    setSeenLines(buffer.lines.length);
    setFollow(false);
  }

  function pauseWhenScrolled() {
    const element = viewport.current;
    if (element && element.scrollHeight - element.clientHeight - element.scrollTop > 4) {
      pauseFollow();
    }
  }

  function showPage(nextPage: number) {
    setSelectedPage(nextPage);
    if (follow) setSeenLines(buffer.lines.length);
    setFollow(false);
    if (viewport.current) viewport.current.scrollTop = 0;
  }

  if (!run) {
    return error ? (
      <Feedback retry={retry}>{error}</Feedback>
    ) : (
      <Feedback>正在读取执行记录…</Feedback>
    );
  }

  const action = actions.find((item) => item.id === run.action);
  const target = actions.find((item) => item.id === run.target);
  const showResult = connection === "done";
  const outputStatus = run.logs_expired
    ? "日志已过期。"
    : connection === "reconnecting"
      ? "日志连接已断开，正在重连…"
      : run.status === "queued"
        ? "正在等待执行…"
        : run.started_at === null
          ? "命令未启动。"
          : connection === "loading" || connection === "connecting"
            ? "正在读取日志…"
            : connection === "open" && run.status !== "running"
              ? "正在读取日志…"
              : buffer.lines.length === 0 && connection === "done"
                ? "本次执行没有输出。"
                : buffer.lines.length === 0 && connection === "open"
                  ? "正在等待输出…"
                  : "";

  return (
    <section className={`run-panel run-panel-${mode}`} aria-labelledby="run-heading">
      <header className="run-heading">
        <div>
          <div className="heading-with-status">
            <h2 id="run-heading">{mode === "actions" ? "本次执行" : run.label}</h2>
            <span aria-live="polite">
              <StatusBadge status={run.status} />
            </span>
          </div>
          <div className="run-meta">
            {run.target && <span>{run.target}</span>}
            <span>{sources[run.source]}</span>
            <span>
              提交于 <RunTime timestamp={run.submitted_at} />
            </span>
            {run.started_at !== null && run.started_at !== run.submitted_at && (
              <span>
                开始于 <RunTime timestamp={run.started_at} />
              </span>
            )}
          </div>
        </div>
        {mode === "actions" && (
          <Link className="text-link" href={historyHref(run.id)}>
            查看执行记录
          </Link>
        )}
      </header>
      {mode === "history" && Object.keys(run.selections).length > 0 && (
        <dl className="run-selections">
          {Object.entries(run.selections).map(([key, value]) => {
            const selection = selectionLabel(action, key, value, target);
            return (
              <div key={key}>
                <dt>{selection.label}</dt>
                <dd>{selection.value}</dd>
              </div>
            );
          })}
        </dl>
      )}
      <div className="log-toolbar">
        <h3 id="log-heading">执行日志</h3>
        {hasLogs && (
          <label className="follow-toggle">
            <span>跟随输出</span>
            <input
              type="checkbox"
              role="switch"
              checked={follow}
              aria-checked={follow}
              onChange={toggleFollow}
            />
            <span className="switch-visual" aria-hidden="true" />
          </label>
        )}
      </div>
      <section
        className="log-window"
        ref={viewport}
        onScroll={pauseWhenScrolled}
        // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- The log scroll area must be keyboard-focusable.
        tabIndex={0}
        aria-labelledby="log-heading"
      >
        {lines.length > 0 && hasLogs && (
          <div className="log-lines">
            {lines.map((line, index) => (
              <div className="log-line" key={page * linesPerPage + index}>
                <time dateTime={dateTimeAttribute(line.at)}>{formatTime(line.at)}</time>
                <span className="log-text">
                  {line.parts.map((part, partIndex) => (
                    <span
                      key={partIndex}
                      className={part.stream === "stderr" ? "log-stderr" : undefined}
                    >
                      {part.text}
                    </span>
                  ))}
                </span>
              </div>
            ))}
          </div>
        )}
        {outputStatus && <output className="log-note">{outputStatus}</output>}
        {error && <Feedback retry={retry}>{error}</Feedback>}
      </section>
      {hasLogs && (lastPage > 0 || newLines > 0) && (
        <fieldset className="log-navigation" aria-label="日志分页">
          {lastPage > 0 && (
            <>
              <button
                className="button button-secondary"
                disabled={page === 0}
                onClick={() => showPage(page - 1)}
              >
                上一段
              </button>
              <span>
                第 {page + 1} / {lastPage + 1} 段
              </span>
              <button
                className="button button-secondary"
                disabled={page === lastPage}
                onClick={() => showPage(page + 1)}
              >
                下一段
              </button>
            </>
          )}
          {newLines > 0 && (
            <button className="button button-secondary" onClick={() => setFollow(true)}>
              {newLines} 行新输出
            </button>
          )}
        </fieldset>
      )}
      {showResult && (run.exit_code !== null || run.error || run.finished_at !== null) && (
        <output className="run-result">
          {run.started_at !== null && run.finished_at !== null && (
            <span>耗时 {formatDuration(run.started_at, run.finished_at)}</span>
          )}
          {run.exit_code !== null && (
            <span>
              退出码 <code>{run.exit_code}</code>
            </span>
          )}
          {run.error && <span className="result-error">{run.error}</span>}
        </output>
      )}
    </section>
  );
}
