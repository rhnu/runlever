import { useLayoutEffect, useRef, useState } from "react";
import { api, ApiError, isUnauthorized } from "./api";
import type { Action } from "./model";
import { actionHref, Link } from "./route";
import { RunDrawer } from "./RunDrawer";
import { Feedback, Icon } from "./ui";

interface ActionsPageProps {
  actions: Action[];
  actionId?: string;
  run?: number;
  currentRuns: ReadonlyMap<string, number>;
  onRunStarted: (actionId: string, run: number) => void;
  onUnauthorized: () => void;
}

export function ActionsPage({
  actions,
  actionId,
  run,
  currentRuns,
  onRunStarted,
  onUnauthorized,
}: ActionsPageProps) {
  const selectedId = actions.find((action) => action.id === actionId)?.id;
  const previousTarget = useRef<{ actionId: string | undefined; run: number | undefined }>(null);

  useLayoutEffect(() => {
    const previous = previousTarget.current;
    previousTarget.current = { actionId: selectedId, run };
    // Opening or closing a run should keep the list at its current scroll position.
    if (
      selectedId &&
      (!previous ||
        (previous.actionId !== selectedId && previous.run === undefined && run === undefined))
    ) {
      document.getElementById(`action-row-${selectedId}`)?.scrollIntoView({ block: "nearest" });
    }
  }, [selectedId, run]);

  const groups = new Map<string | undefined, { label: string; items: Action[] }>();
  for (const action of actions) {
    const id = action.group?.id;
    const group = groups.get(id);
    if (group) group.items.push(action);
    else groups.set(id, { label: action.group?.label ?? "未分组", items: [action] });
  }

  return (
    <>
      {actionId && !selectedId ? (
        <Feedback>此动作已不在当前配置中。</Feedback>
      ) : actions.length === 0 ? (
        <Feedback>还没有配置动作。</Feedback>
      ) : null}
      {groups.size > 0 && (
        <div className="actions-list">
          {[...groups].map(([id, group]) => {
            const heading = id !== undefined || groups.size > 1;
            const headingId = id === undefined ? "actions-ungrouped" : `actions-group-${id}`;
            return (
              <section
                className="action-group"
                key={headingId}
                aria-labelledby={heading ? headingId : undefined}
              >
                {heading && (
                  <h2 className="action-group-title" id={headingId}>
                    {group.label}
                  </h2>
                )}
                <div className="action-columns" aria-hidden="true">
                  <span>动作名称</span>
                  <span>参数</span>
                  <span>运行</span>
                </div>
                <ul className="action-group-list">
                  {group.items.map((action) => (
                    <li
                      key={action.id}
                      id={`action-row-${action.id}`}
                      className="action-item"
                      data-selected={action.id === selectedId || undefined}
                    >
                      <ActionForm
                        action={action}
                        latestRun={currentRuns.get(action.id)}
                        onRunStarted={onRunStarted}
                        onUnauthorized={onUnauthorized}
                      />
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
        </div>
      )}
      {run !== undefined && (
        <RunDrawer
          key={run}
          id={run}
          actions={actions}
          onUnauthorized={onUnauthorized}
          closeHref={actionHref(actionId)}
          returnFocusId={actionId ? `action-run-${actionId}` : undefined}
          showHistoryLink
        />
      )}
    </>
  );
}

interface ActionFormProps {
  action: Action;
  latestRun?: number;
  onRunStarted: ActionsPageProps["onRunStarted"];
  onUnauthorized: ActionsPageProps["onUnauthorized"];
}

function ActionForm({ action, latestRun, onRunStarted, onUnauthorized }: ActionFormProps) {
  const [selections, setSelections] = useState<ReadonlyMap<string, string>>(() => new Map());
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string>();
  const submitting = useRef(false);
  const fields = action.inputs.map((input) => ({
    input,
    value: input.options.find((option) => option.id === selections.get(input.id))?.id ?? "",
  }));
  const ready = fields.every(({ value }) => value !== "");
  const inputsReady = fields.every(({ input }) => input.options.length > 0);
  const problemId = `action-problem-${action.id}`;

  async function execute() {
    if (submitting.current || !ready) return;
    submitting.current = true;
    setBusy(true);
    setProblem(undefined);
    let runId: number;
    try {
      const submitted = Object.fromEntries<string>(
        fields.map(({ input, value }) => [input.id, value]),
      );
      const result = await api.start(action.id, submitted);
      runId = result.run_id;
    } catch (error) {
      if (isUnauthorized(error)) onUnauthorized();
      else if (error instanceof ApiError && error.status === 429) {
        setProblem(error.detail.error ?? "等待队列已满，请稍后再试。");
      } else if (error instanceof ApiError && [400, 403, 404, 422].includes(error.status)) {
        setProblem(error.detail.error ?? "服务器拒绝了请求，请检查所选参数和动作配置。");
      } else {
        setProblem("未能确认执行请求，请查看执行记录。");
      }
      return;
    } finally {
      submitting.current = false;
      setBusy(false);
    }
    onRunStarted(action.id, runId);
  }

  // Require the Run button so using Enter in a select cannot submit the action.
  return (
    <form
      className="action-row"
      onSubmit={(event) => event.preventDefault()}
      aria-labelledby={`action-name-${action.id}`}
      aria-describedby={problem || !inputsReady ? problemId : undefined}
      aria-busy={busy}
    >
      <span className="action-name" id={`action-name-${action.id}`}>
        {action.label}
      </span>
      {fields.length > 0 ? (
        <div className="action-parameters">
          {fields.map(({ input, value }) => (
            <label className="enum-field" key={input.id}>
              <span>{input.label}</span>
              <select
                className="select"
                value={value}
                disabled={busy || input.options.length === 0}
                onChange={(event) => {
                  const selected = event.currentTarget.value;
                  setSelections((previous) => new Map(previous).set(input.id, selected));
                }}
                required
              >
                <option value="" disabled>
                  请选择
                </option>
                {input.options.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
          ))}
        </div>
      ) : (
        <span className="no-parameters" aria-label="无需参数">
          —
        </span>
      )}
      <div className="action-controls">
        {latestRun !== undefined && (
          <Link
            className="action-latest-run"
            href={actionHref(action.id, latestRun)}
            aria-label={`查看「${action.label}」最近一次运行`}
            title="查看最近一次运行"
          >
            <Icon name="terminal" />
          </Link>
        )}
        <button
          className={fields.length ? "button button-primary" : "button button-secondary"}
          id={`action-run-${action.id}`}
          type="button"
          disabled={busy || !ready}
          aria-label={busy ? `正在提交「${action.label}」` : `运行「${action.label}」`}
          onClick={() => void execute()}
        >
          <Icon name="play" />
          {busy ? "正在提交…" : "运行"}
        </button>
      </div>
      {(!inputsReady || problem) && (
        <p className="action-problem field-error" id={problemId} role="alert">
          {!inputsReady ? "有参数缺少可选值，请检查动作配置。" : problem}
        </p>
      )}
    </form>
  );
}
