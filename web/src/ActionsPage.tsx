import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { api, ApiError, isUnauthorized } from "./api";
import type { Action, ActionInput } from "./model";
import { actionHref, historyHref, Link } from "./route";
import { Feedback, RunPanel } from "./RunPanel";

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
  const action = actionId ? actions.find((item) => item.id === actionId) : actions[0];
  const runId = run ?? (action && currentRuns.get(action.id));
  const mobileBack = useRef<HTMLAnchorElement>(null);
  const showDetail = Boolean(actionId || run);

  useEffect(() => {
    if (showDetail && matchMedia("(max-width: 760px)").matches) mobileBack.current?.focus();
  }, [showDetail]);

  if (actions.length === 0 && runId === undefined) {
    return <Feedback>还没有配置动作。</Feedback>;
  }

  return (
    <div className="split split-actions" data-open={showDetail}>
      <nav className="record-list" aria-label="动作列表">
        <ul>
          {actions.map((item) => (
            <li key={item.id}>
              <Link
                className="record-row"
                href={actionHref(
                  item.id,
                  (item.id === actionId ? run : undefined) ?? currentRuns.get(item.id),
                )}
                aria-current={action?.id === item.id ? "page" : undefined}
              >
                <span className="record-title">{item.label}</span>
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <div className="detail">
        <Link className="mobile-back text-link" href={actionHref()} ref={mobileBack}>
          ← 返回动作列表
        </Link>
        {action ? (
          <ActionForm
            key={action.id}
            action={action}
            onRunStarted={onRunStarted}
            onUnauthorized={onUnauthorized}
          />
        ) : (
          <Feedback>此动作已不在当前配置中。</Feedback>
        )}
        {runId !== undefined && (
          <div className="action-result">
            <RunPanel
              key={runId}
              id={runId}
              mode="actions"
              actions={actions}
              onUnauthorized={onUnauthorized}
            />
          </div>
        )}
      </div>
    </div>
  );
}

interface ActionFormProps {
  action: Action;
  onRunStarted: ActionsPageProps["onRunStarted"];
  onUnauthorized: ActionsPageProps["onUnauthorized"];
}

function ActionForm({ action, onRunStarted, onUnauthorized }: ActionFormProps) {
  const [selections, setSelections] = useState<ReadonlyMap<string, string>>(() => new Map());
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<{ message: string; run?: number }>();
  const inputsReady = action.inputs.every((input) => input.options.length > 0);
  const choiceLabels = action.inputs.map(
    (input) => input.options.find((option) => option.id === selections.get(input.id))?.label,
  );
  const ready = inputsReady && choiceLabels.every((label) => label !== undefined);
  let caption = action.label;
  if (ready && choiceLabels.length === 1) caption = `${action.label} ${choiceLabels[0]}`;
  else if (ready && choiceLabels.length > 1) caption = choiceLabels.join(" ");
  const fields = action.inputs.map((input) => ({
    input,
    compact:
      input.options.length >= 3 &&
      input.options.length <= 5 &&
      input.options.every(({ label }) => label.length <= 12),
  }));

  async function execute(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy || !ready) return;
    setBusy(true);
    setProblem(undefined);
    try {
      const { run_id } = await api.start(action.id, Object.fromEntries(selections));
      onRunStarted(action.id, run_id);
    } catch (error) {
      if (isUnauthorized(error)) onUnauthorized();
      else if (error instanceof ApiError && error.status === 409) {
        setProblem({ message: "该目标正在执行其他动作。", run: error.detail.run_id });
      } else if (error instanceof ApiError && [400, 403, 404, 422].includes(error.status)) {
        setProblem({
          message: error.detail.error ?? "服务器拒绝了请求，请检查所选参数和动作配置。",
        });
      } else {
        setProblem({ message: "未能确认执行请求，请查看执行记录。" });
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-labelledby="action-heading">
      <h2 className="detail-title" id="action-heading">
        {action.label}
      </h2>
      <form className="action-form" onSubmit={(event) => void execute(event)} aria-busy={busy}>
        {fields.map(({ input, compact }) => (
          <EnumField
            key={input.id}
            input={input}
            compact={compact}
            disabled={busy}
            value={selections.get(input.id) ?? ""}
            onChange={(value) =>
              setSelections((previous) => new Map(previous).set(input.id, value))
            }
          />
        ))}
        {!inputsReady && (
          <p className="field-error" role="alert">
            有参数缺少可选值，请检查动作配置。
          </p>
        )}
        <div className="form-actions">
          <button
            className="button button-primary"
            disabled={busy || !ready}
            aria-label={
              busy || caption === action.label ? undefined : `${caption}，${action.label}`
            }
          >
            {busy ? "正在提交…" : caption}
          </button>
        </div>
      </form>
      {problem && (
        <div className="action-problem" role="alert">
          <span>{problem.message}</span>
          {problem.run !== undefined && (
            <Link className="text-link" href={historyHref(problem.run)}>
              查看正在执行的动作
            </Link>
          )}
        </div>
      )}
    </section>
  );
}

interface EnumFieldProps {
  input: ActionInput;
  value: string;
  disabled: boolean;
  compact: boolean;
  onChange: (value: string) => void;
}

function EnumField({ input, value, disabled, compact, onChange }: EnumFieldProps) {
  if (!compact) {
    return (
      <label className="enum-field">
        <span>{input.label}</span>
        <select
          className="select"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          disabled={disabled}
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
    );
  }

  return (
    <fieldset className="enum-field" disabled={disabled}>
      <legend>{input.label}</legend>
      <div className="enum-options">
        {input.options.map((option) => (
          <label key={option.id} className="enum-option">
            <input
              type="radio"
              name={input.id}
              value={option.id}
              checked={value === option.id}
              onChange={(event) => onChange(event.target.value)}
              required
            />
            <span>{option.label}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}
