import { useCallback, useEffect, useRef, useState } from "react";
import { api, isUnauthorized } from "./api";
import { formatDuration, matchesFilters, selectionLabel, sources, statuses } from "./model";
import type { Action, Filters, Run } from "./model";
import { historyHref, Link, navigate } from "./route";
import { Feedback, RunPanel, RunTime, StatusBadge } from "./RunPanel";

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
  const [defaultRun, setDefaultRun] = useState<number>();
  const [more, setMore] = useState(false);
  const [pending, setPending] = useState(true);
  const [fault, setFault] = useState<"initial" | "more">();
  const [request, setRequest] = useState<{ before?: number }>({});
  const [seenActions, setSeenActions] = useState<Record<string, string>>({});
  const selected = run ?? defaultRun;
  const mobileBack = useRef<HTMLAnchorElement>(null);

  const { action, source, status } = filters;
  useEffect(() => {
    const controller = new AbortController();
    const { before } = request;
    api.runs({ action, source, status }, before, controller.signal).then(
      (page) => {
        if (controller.signal.aborted) return;
        setSeenActions((previous) => ({
          ...previous,
          ...Object.fromEntries(page.map(({ action }) => [action, action])),
        }));
        setRecords((previous) =>
          before === undefined
            ? page
            : [...previous, ...page.filter((item) => !previous.some(({ id }) => id === item.id))],
        );
        setDefaultRun((previous) => previous ?? page[0]?.id);
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

  useEffect(() => {
    if (run && matchMedia("(max-width: 760px)").matches) mobileBack.current?.focus();
  }, [run]);

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
  const actionById = new Map(actions.map((item) => [item.id, item]));
  if (action) actionOptions.set(action, actionOptions.get(action) ?? action);
  for (const item of actions) actionOptions.set(item.id, item.label);
  const hasFilters = Boolean(action || status || source);

  return (
    <div className="history-content" data-open={Boolean(run)}>
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
          disabled={pending}
          onClick={() => load()}
        >
          刷新
        </button>
      </fieldset>
      <div className="split split-history" data-open={Boolean(run)}>
        <nav className="record-list" aria-label="执行记录列表" aria-busy={pending}>
          <ul>
            {records.map((item) => {
              const action = actionById.get(item.action);
              const target = actionById.get(item.target);
              const choices = Object.entries(item.selections).map(
                ([key, value]) => selectionLabel(action, key, value, target).value,
              );
              const description = choices
                .filter((choice) => !item.label.includes(choice))
                .join(" / ");
              return (
                <li key={item.id}>
                  <Link
                    className="record-row"
                    aria-current={selected === item.id ? "page" : undefined}
                    href={historyHref(item.id, filters)}
                  >
                    <div className="record-summary">
                      <span className="record-title">{item.label}</span>
                      {description && <span className="record-choices">{description}</span>}
                      <div className="record-meta">
                        <RunTime timestamp={item.started_at} />
                        {item.finished_at !== null && (
                          <span>{formatDuration(item.started_at, item.finished_at)}</span>
                        )}
                      </div>
                    </div>
                    <StatusBadge status={item.status} />
                  </Link>
                </li>
              );
            })}
          </ul>
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
                disabled={pending}
                onClick={() => load(records.at(-1)?.id)}
              >
                {pending ? "正在加载…" : "加载更早的记录"}
              </button>
            </div>
          )}
        </nav>
        <div className="detail">
          <Link
            className="mobile-back text-link"
            href={historyHref(undefined, filters)}
            ref={mobileBack}
          >
            ← 返回执行记录
          </Link>
          {selected !== undefined ? (
            <RunPanel
              key={selected}
              id={selected}
              mode="history"
              actions={actions}
              onUnauthorized={onUnauthorized}
              onRun={updateRun}
            />
          ) : (
            <Feedback>{pending ? "正在读取记录…" : "选择一条记录查看详情。"}</Feedback>
          )}
        </div>
      </div>
    </div>
  );
}
