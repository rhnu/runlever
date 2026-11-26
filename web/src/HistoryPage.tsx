import { useCallback, useEffect, useState } from "react";
import { api, isUnauthorized } from "./api";
import { formatDuration, matchesFilters, sources, statuses } from "./model";
import type { Action, Filters, Run } from "./model";
import { historyHref, Link, navigate } from "./route";
import { RunDrawer } from "./RunDrawer";
import { Feedback, Icon, RunTime, StatusBadge } from "./ui";

const pageSize = 50;

interface HistoryPageProps {
  actions: Action[];
  filters: Filters;
  run?: number;
  onUnauthorized: () => void;
}

export function HistoryPage(props: HistoryPageProps) {
  return <HistoryResults key={JSON.stringify(props.filters)} {...props} />;
}

function HistoryResults({ actions, filters, run, onUnauthorized }: HistoryPageProps) {
  const [records, setRecords] = useState<Run[]>([]);
  const [more, setMore] = useState(false);
  const [pending, setPending] = useState(true);
  const [fault, setFault] = useState<"initial" | "more">();
  const [request, setRequest] = useState<{ before?: number }>({});
  const [seenActions, setSeenActions] = useState<Record<string, string>>({});

  const { action, source, status } = filters;
  useEffect(() => {
    const controller = new AbortController();
    const { before } = request;
    api.runs({ action, source, status }, before, controller.signal).then(
      (page) => {
        if (controller.signal.aborted) return;
        setSeenActions((previous) => ({
          ...previous,
          ...Object.fromEntries(page.map(({ action, label }) => [action, label])),
        }));
        setRecords((previous) =>
          before === undefined
            ? page
            : [...previous, ...page.filter((item) => !previous.some(({ id }) => id === item.id))],
        );
        setMore(page.length === pageSize);
        setPending(false);
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        if (isUnauthorized(error)) onUnauthorized();
        else setFault(before === undefined ? "initial" : "more");
        setPending(false);
      },
    );
    return () => controller.abort();
  }, [action, source, status, request, onUnauthorized]);

  const updateRun = useCallback(
    (next: Run) => {
      setRecords((previous) => {
        if (!previous.some(({ id }) => id === next.id)) return previous;
        return matchesFilters(next, { action, source, status })
          ? previous.map((item) => (item.id === next.id ? next : item))
          : previous.filter((item) => item.id !== next.id);
      });
    },
    [action, source, status],
  );

  function load(before?: number) {
    setFault(undefined);
    setPending(true);
    setRequest({ before });
  }

  function setFilter<K extends keyof Filters>(key: K, value: Filters[K]) {
    navigate(historyHref(undefined, { ...filters, [key]: value }));
  }

  const actionOptions = new Map(Object.entries(seenActions));
  if (action) actionOptions.set(action, actionOptions.get(action) ?? action);
  for (const item of actions) actionOptions.set(item.id, item.label);
  const hasFilters = Boolean(action || status || source);

  return (
    <div className="history-content">
      <fieldset className="filters" aria-label="筛选执行记录">
        <label>
          动作
          <select
            className="select"
            value={action}
            onChange={(event) => setFilter("action", event.target.value)}
          >
            <option value="">全部</option>
            {[...actionOptions].map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label>
          状态
          <select
            className="select"
            value={status}
            onChange={(event) => setFilter("status", event.target.value as Filters["status"])}
          >
            <option value="">全部</option>
            {Object.entries(statuses).map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label>
          来源
          <select
            className="select"
            value={source}
            onChange={(event) => setFilter("source", event.target.value as Filters["source"])}
          >
            <option value="">全部</option>
            {Object.entries(sources).map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <button
          className="button button-secondary refresh"
          type="button"
          disabled={pending}
          onClick={() => load()}
        >
          <Icon name="refresh" />
          刷新
        </button>
      </fieldset>
      <section className="record-list" aria-label="执行记录列表" aria-busy={pending}>
        {records.length > 0 && (
          <>
            <div className="record-head" aria-hidden="true">
              <span className="record-title">动作</span>
              <span className="record-choices">参数</span>
              <span className="record-status">状态</span>
              <span className="record-source">来源</span>
              <span className="record-submitted">提交时间</span>
              <span className="record-duration">耗时</span>
              <span className="record-chevron" />
            </div>
            <ul>
              {records.map((item) => {
                const choices = Object.values(item.selection_labels)
                  .map(({ value }) => value)
                  .join(" / ");
                const duration =
                  item.started_at !== null && item.finished_at !== null
                    ? formatDuration(item.started_at, item.finished_at)
                    : "—";
                return (
                  <li key={item.id}>
                    <Link
                      id={`history-run-${item.id}`}
                      className="record-row"
                      aria-current={run === item.id ? "page" : undefined}
                      href={historyHref(item.id, filters)}
                    >
                      <span className="record-title" title={item.label}>
                        {item.label}
                      </span>
                      <span className="record-choices" title={choices || undefined}>
                        {choices || "—"}
                      </span>
                      <span className="record-status">
                        <StatusBadge status={item.status} />
                      </span>
                      <span className="record-source">{sources[item.source]}</span>
                      <span className="record-submitted">
                        <RunTime timestamp={item.submitted_at} />
                      </span>
                      <span className="record-duration">{duration}</span>
                      <span className="record-chevron" aria-hidden="true">
                        <Icon name="chevron-right" />
                      </span>
                    </Link>
                  </li>
                );
              })}
            </ul>
          </>
        )}
        {pending && records.length === 0 && <Feedback>正在读取执行记录…</Feedback>}
        {fault === "initial" && <Feedback retry={() => load()}>无法读取执行记录。</Feedback>}
        {fault === "more" && (
          <Feedback retry={() => load(request.before)}>无法读取较早的执行记录。</Feedback>
        )}
        {!pending && !fault && records.length === 0 && (
          <Feedback>{hasFilters ? "没有符合条件的执行记录。" : "还没有执行记录。"}</Feedback>
        )}
        {more && !fault && records.length > 0 && (
          <div className="load-more">
            <button
              className="button button-secondary"
              type="button"
              disabled={pending}
              onClick={() => load(records.at(-1)?.id)}
            >
              {pending ? "正在加载…" : "加载更早的记录"}
            </button>
          </div>
        )}
      </section>
      {run !== undefined && (
        <RunDrawer
          key={run}
          id={run}
          onUnauthorized={onUnauthorized}
          onRun={updateRun}
          closeHref={historyHref(undefined, filters)}
          returnFocusId={`history-run-${run}`}
        />
      )}
    </div>
  );
}
