import { useId, useLayoutEffect, useRef, useState } from "react";
import { dateTimeAttribute, formatDuration, formatTime, selectionLabel, sources } from "./model";
import type { Action, Run } from "./model";
import { closeRun, historyHref, Link } from "./route";
import { Feedback, Icon, RunTime, StatusBadge } from "./ui";
import { useRunStream } from "./use-run-stream";

const linesPerPage = 400;

interface RunDrawerProps {
  id: number;
  actions: Action[];
  onUnauthorized: () => void;
  onRun?: (run: Run) => void;
  closeHref: string;
  returnFocusId?: string;
  showHistoryLink?: boolean;
}

export function RunDrawer({
  id,
  actions,
  onUnauthorized,
  onRun,
  closeHref,
  returnFocusId,
  showHistoryLink,
}: RunDrawerProps) {
  const { run, ...stream } = useRunStream(id, onUnauthorized, onRun);
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const closing = useRef(false);
  const heading = useId();
  const missing = stream.connection === "missing";

  useLayoutEffect(() => {
    const element = dialog.current;
    if (!element) return;
    element.showModal();
    closeButton.current?.focus({ preventScroll: true });
    return () => {
      element.close();
      // The trigger may have been removed by a filter or this navigation may open another drawer.
      requestAnimationFrame(() => {
        if (
          document.querySelector("dialog.run-drawer[open]") ||
          document.activeElement !== document.body
        )
          return;
        if (returnFocusId) document.getElementById(returnFocusId)?.focus({ preventScroll: true });
        if (document.activeElement === document.body) {
          document.getElementById("main-content")?.focus({ preventScroll: true });
        }
      });
    };
  }, [returnFocusId]);

  function dismiss() {
    if (closing.current) return;
    closing.current = true;
    closeRun(closeHref);
  }

  return (
    <dialog
      className="run-drawer"
      ref={dialog}
      aria-labelledby={heading}
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
    >
      <header className="run-heading">
        <div className="heading-with-status">
          <h2 id={heading}>{run?.label ?? "执行详情"}</h2>
          <span aria-live="polite">{run && !missing && <StatusBadge status={run.status} />}</span>
        </div>
        <div className="run-drawer-actions">
          {showHistoryLink && run && !missing && (
            <Link
              className="text-link history-link"
              href={historyHref(run.id)}
              title="查看执行记录"
            >
              执行记录 <Icon name="arrow-up-right" />
            </Link>
          )}
          <button
            className="button button-secondary run-drawer-close"
            type="button"
            aria-label="关闭执行详情"
            ref={closeButton}
            onClick={dismiss}
          >
            <Icon name="x" />
          </button>
        </div>
      </header>
      <div className="run-drawer-body">
        {missing ? (
          <p className="feedback feedback-error" role="alert">
            {stream.error}
          </p>
        ) : run ? (
          <RunDetails run={run} actions={actions} stream={stream} />
        ) : stream.error ? (
          <Feedback retry={stream.retry}>{stream.error}</Feedback>
        ) : (
          <Feedback>正在读取执行记录…</Feedback>
        )}
      </div>
    </dialog>
  );
}

type RunStream = Omit<ReturnType<typeof useRunStream>, "run">;

function RunDetails({ run, actions, stream }: { run: Run; actions: Action[]; stream: RunStream }) {
  const { buffer, error, revision, retry } = stream;
  const logHeading = useId();
  const [follow, setFollow] = useState(true);
  const [selectedPage, setSelectedPage] = useState(0);
  const [seenLines, setSeenLines] = useState(0);
  const viewport = useRef<HTMLElement>(null);
  const lastPage = Math.max(0, Math.ceil(buffer.lines.length / linesPerPage) - 1);
  const page = follow ? lastPage : Math.min(lastPage, selectedPage);
  const lines = buffer.lines.slice(page * linesPerPage, (page + 1) * linesPerPage);
  const newLines = follow ? 0 : buffer.lines.length - seenLines;
  const hasLogs = run.started_at !== null && !run.logs_expired;
  const outputStatus = describeOutput(run, stream);
  const duration =
    run.started_at !== null && run.finished_at !== null
      ? formatDuration(run.started_at, run.finished_at)
      : undefined;

  useLayoutEffect(() => {
    if (follow && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  }, [follow, revision, hasLogs, outputStatus]);

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

  const action = actions.find((item) => item.id === run.action);
  const target = actions.find((item) => item.id === run.target);

  return (
    <>
      <dl className="run-meta">
        {Object.entries(run.selections).map(([key, value]) => {
          const selection = selectionLabel(action, key, value, target);
          return (
            <div key={key}>
              <dt>{selection.label}</dt>
              <dd>{selection.value}</dd>
            </div>
          );
        })}
        {run.target && run.target !== run.action && (
          <div>
            <dt>目标</dt>
            <dd>{target?.label ?? run.target}</dd>
          </div>
        )}
        <div>
          <dt>来源</dt>
          <dd>{sources[run.source]}</dd>
        </div>
        <div>
          <dt>提交于</dt>
          <dd>
            <RunTime timestamp={run.submitted_at} />
          </dd>
        </div>
        {run.started_at !== null && run.started_at !== run.submitted_at && (
          <div>
            <dt>开始于</dt>
            <dd>
              <RunTime timestamp={run.started_at} />
            </dd>
          </div>
        )}
      </dl>
      <div className="log-toolbar">
        <h3 id={logHeading}>
          <Icon name="terminal" /> 执行日志
        </h3>
        {hasLogs && (
          <label className="follow-toggle">
            <span>跟随输出</span>
            <input
              type="checkbox"
              role="switch"
              aria-checked={follow}
              checked={follow}
              onChange={toggleFollow}
            />
            <span className="switch-visual" aria-hidden="true" />
          </label>
        )}
      </div>
      {error && <Feedback retry={retry}>{error}</Feedback>}
      <section
        className="log-window"
        ref={viewport}
        onScroll={pauseWhenScrolled}
        // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- The log scroll area must be keyboard-focusable.
        tabIndex={0}
        aria-labelledby={logHeading}
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
      {(run.exit_code !== null || run.error || duration) && (
        <output className="run-result">
          {duration && <span>耗时 {duration}</span>}
          {run.exit_code !== null && (
            <span>
              退出码 <code>{run.exit_code}</code>
            </span>
          )}
          {run.error && <span className="result-error">{run.error}</span>}
        </output>
      )}
    </>
  );
}

function describeOutput(run: Run, { connection, buffer }: RunStream) {
  if (run.logs_expired) return "日志已过期。";
  if (connection === "reconnecting") return "日志连接已断开，正在重连…";
  if (run.status === "queued") return "正在等待执行…";
  if (run.started_at === null) return "命令未启动。";
  if (connection === "loading" || connection === "connecting") return "正在读取日志…";
  if (buffer.lines.length > 0) return;
  if (connection === "done") return "本次执行没有输出。";
  if (connection === "open") {
    return run.status === "running" ? "正在等待输出…" : "正在读取日志…";
  }
}
