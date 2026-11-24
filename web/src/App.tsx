import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
import { api, ApiError, isUnauthorized } from "./api";
import { ActionsPage } from "./ActionsPage";
import { HistoryPage } from "./HistoryPage";
import type { Action } from "./model";
import { actionHref, historyHref, Link, navigate, readRoute, useRoute } from "./route";
import { Feedback } from "./ui";

type Session =
  | { state: "checking" }
  | { state: "locked"; expired: boolean }
  | { state: "ready"; actions: Action[] }
  | { state: "failed" };

export function App() {
  const route = useRoute();
  const [session, setSession] = useState<Session>({ state: "checking" });
  const [sessionCheck, setSessionCheck] = useState(0);
  const [logoutPending, setLogoutPending] = useState(false);
  const [logoutError, setLogoutError] = useState(false);
  const [currentRuns, setCurrentRuns] = useState<ReadonlyMap<string, number>>(() => {
    const initial = readRoute();
    return initial.view === "actions" && initial.action && initial.run
      ? new Map([[initial.action, initial.run]])
      : new Map();
  });
  const view = route.view;
  const pageTitle = view === "actions" ? "动作" : "执行记录";

  const expire = useCallback(() => {
    setSession({ state: "locked", expired: true });
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    api.actions(controller.signal).then(
      (actions) => {
        if (!controller.signal.aborted) setSession({ state: "ready", actions });
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        if (isUnauthorized(error)) setSession({ state: "locked", expired: false });
        else setSession({ state: "failed" });
      },
    );
    return () => controller.abort();
  }, [sessionCheck]);

  useEffect(() => {
    document.title = session.state === "ready" ? `${pageTitle} — Runlever` : "Runlever";
  }, [pageTitle, session.state]);

  function checkSession() {
    setSession({ state: "checking" });
    setSessionCheck((previous) => previous + 1);
  }

  async function logout() {
    setLogoutPending(true);
    setLogoutError(false);
    try {
      await api.logout();
    } catch (error) {
      if (!isUnauthorized(error)) {
        setLogoutError(true);
        return;
      }
    } finally {
      setLogoutPending(false);
    }
    setSession({ state: "locked", expired: false });
    setCurrentRuns(new Map());
    navigate(actionHref(), true);
  }

  function started(action: string, run: number) {
    setCurrentRuns((previous) => new Map(previous).set(action, run));
    const current = readRoute();
    // Another accepted run is still available on its row if the user is reading a drawer.
    if (current.view === "actions" && current.run === undefined) {
      navigate(actionHref(action, run));
    }
  }

  if (session.state === "locked") return <Login expired={session.expired} onLogin={checkSession} />;
  if (session.state === "checking" || session.state === "failed") {
    return (
      <main className="login-layout">
        <div className="login-card">
          <Brand />
          {session.state === "checking" ? (
            <Feedback>正在连接 Runlever…</Feedback>
          ) : (
            <Feedback retry={checkSession}>无法连接 Runlever。</Feedback>
          )}
        </div>
      </main>
    );
  }

  return (
    <div className="app-layout">
      <a className="skip-link" href="#main-content">
        跳到主要内容
      </a>
      <header className="app-header">
        <div className="header-inner">
          <Brand />
          <nav className="primary-nav" aria-label="主导航">
            <Link href={actionHref()} aria-current={view === "actions" ? "page" : undefined}>
              动作
            </Link>
            <Link href={historyHref()} aria-current={view === "history" ? "page" : undefined}>
              执行记录
            </Link>
          </nav>
          <div className="account">
            <button
              className="button button-quiet"
              disabled={logoutPending}
              onClick={() => void logout()}
            >
              {logoutPending ? "正在退出…" : "退出"}
            </button>
            {logoutError && (
              <p className="field-error" role="alert">
                退出失败，请重试。
              </p>
            )}
          </div>
        </div>
      </header>
      <main className={`main-content page-${view}`} id="main-content" tabIndex={-1}>
        <header className="page-heading">
          <h1>{pageTitle}</h1>
        </header>
        {route.view === "actions" ? (
          <ActionsPage
            actions={session.actions}
            actionId={route.action}
            run={route.run}
            currentRuns={currentRuns}
            onRunStarted={started}
            onUnauthorized={expire}
          />
        ) : (
          <HistoryPage
            actions={session.actions}
            filters={route.filters}
            run={route.run}
            onUnauthorized={expire}
          />
        )}
      </main>
    </div>
  );
}

function Brand() {
  return (
    <div className="brand">
      <svg className="brand-mark" viewBox="0 0 26 26" aria-hidden="true" fill="currentColor">
        <path d="M2 23h21v-2H2zM5 20v-2a3 3 0 0 1 3-3h5a3 3 0 0 1 3 3v2z" />
        <path d="m10 17 11-12" fill="none" stroke="currentColor" strokeWidth="2" />
        <circle cx="21" cy="5" r="3" />
      </svg>
      Runlever
    </div>
  );
}

function Login({ expired, onLogin }: { expired: boolean; onLogin: () => void }) {
  const [token, setToken] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    setPending(true);
    setError("");
    try {
      await api.login(token);
      onLogin();
    } catch (error) {
      if (isUnauthorized(error)) setError("访问令牌无效。");
      else if (error instanceof ApiError && error.status === 403 && error.detail.error) {
        setError(error.detail.error);
      } else setError("登录失败，请重试。");
    } finally {
      setPending(false);
    }
  }

  return (
    <main className="login-layout">
      <div className="login-card">
        <Brand />
        <h1>登录</h1>
        {expired && <output className="login-message">会话已过期，请重新登录。</output>}
        <form className="login-form" onSubmit={(event) => void submit(event)} aria-busy={pending}>
          <label htmlFor="token">访问令牌</label>
          <input
            id="token"
            type="password"
            autoComplete="current-password"
            required
            value={token}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? "login-error" : undefined}
            onChange={(event) => setToken(event.target.value)}
            readOnly={pending}
          />
          {error && (
            <p id="login-error" className="field-error" role="alert">
              {error}
            </p>
          )}
          <button className="button button-primary" disabled={pending}>
            {pending ? "正在登录…" : "登录"}
          </button>
        </form>
      </div>
    </main>
  );
}
