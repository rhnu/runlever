import { useEffect, useState } from "react";
import type { ComponentPropsWithRef, MouseEvent } from "react";
import { sources, statuses } from "./model";
import type { Filters } from "./model";

type Route =
  | { view: "actions"; action?: string; run?: number }
  | { view: "history"; filters: Filters; run?: number };

export function readRoute(): Route {
  const params = new URLSearchParams(location.search);
  const rawRun = params.get("run");
  const number = rawRun && /^\d+$/.test(rawRun) ? Number(rawRun) : undefined;
  const run = number && Number.isSafeInteger(number) && number > 0 ? number : undefined;
  const action = params.get("action") || "";
  if (params.get("view") === "history") {
    const source = params.get("source") ?? "";
    const status = params.get("status") ?? "";
    return {
      view: "history",
      filters: {
        action,
        source: Object.hasOwn(sources, source) ? (source as Filters["source"]) : "",
        status: Object.hasOwn(statuses, status) ? (status as Filters["status"]) : "",
      },
      run,
    };
  }
  return { view: "actions", action: action || undefined, run };
}

export function actionHref(action?: string, run?: number) {
  const params = new URLSearchParams();
  if (action) params.set("action", action);
  if (run !== undefined) params.set("run", String(run));
  return href(params);
}

export function historyHref(run?: number, filters?: Filters) {
  const params = new URLSearchParams({ view: "history" });
  if (filters) {
    for (const [key, value] of Object.entries(filters)) {
      if (value) params.set(key, value);
    }
  }
  if (run !== undefined) params.set("run", String(run));
  return href(params);
}

function href(params: URLSearchParams) {
  return `${location.pathname}${params.size ? `?${params}` : ""}`;
}

export function navigate(url: string, replace = false) {
  if (url === `${location.pathname}${location.search}`) return;
  if (replace) history.replaceState(null, "", url);
  else history.pushState(null, "", url);
  dispatchEvent(new PopStateEvent("popstate"));
}

export function useRoute() {
  const [route, setRoute] = useState(readRoute);
  useEffect(() => {
    const handlePop = () => setRoute(readRoute());
    addEventListener("popstate", handlePop);
    return () => removeEventListener("popstate", handlePop);
  }, []);
  return route;
}

export function Link({ href, onClick, children, ...props }: ComponentPropsWithRef<"a">) {
  function follow(event: MouseEvent<HTMLAnchorElement>) {
    onClick?.(event);
    if (
      href &&
      !event.defaultPrevented &&
      event.button === 0 &&
      !event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.shiftKey
    ) {
      event.preventDefault();
      navigate(href);
    }
  }
  return (
    <a {...props} href={href} onClick={follow}>
      {children}
    </a>
  );
}
