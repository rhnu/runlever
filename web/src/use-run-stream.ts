import { useEffect, useRef, useState } from "react";
import { api, ApiError, isUnauthorized } from "./api";
import { appendLog, newLogBuffer } from "./log-buffer";
import type { LogChunk, Run } from "./model";

type Connection =
  | "loading"
  | "connecting"
  | "open"
  | "reconnecting"
  | "done"
  | "failed"
  | "missing";

export function useRunStream(
  id: number,
  onUnauthorized: () => void,
  onRun: ((run: Run) => void) | undefined,
) {
  const [buffer] = useState(newLogBuffer);
  const [run, setRun] = useState<Run>();
  const [connection, setConnection] = useState<Connection>("loading");
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const [attempt, setAttempt] = useState(0);
  const onRunRef = useRef(onRun);
  useEffect(() => {
    onRunRef.current = onRun;
  }, [onRun]);

  useEffect(() => {
    const abort = new AbortController();
    let events: EventSource | undefined;
    let recovery: ReturnType<typeof setTimeout> | undefined;
    let frame: number | undefined;

    function stopFeed() {
      events?.close();
      events = undefined;
      clearTimeout(recovery);
    }

    function record(next: Run) {
      setRun(next);
      onRunRef.current?.(next);
    }

    function recordStatus(event: MessageEvent<string>) {
      try {
        record(JSON.parse(event.data) as Run);
        return true;
      } catch {
        fail("无法读取执行状态，请重试。");
        return false;
      }
    }

    function unauthenticated() {
      stopFeed();
      onUnauthorized();
    }

    function fail(message: string, state: "failed" | "missing" = "failed") {
      stopFeed();
      setConnection(state);
      setError(message);
    }

    function recover() {
      if (recovery !== undefined || abort.signal.aborted) return;
      recovery = setTimeout(async () => {
        recovery = undefined;
        const current = events;
        if (!current || abort.signal.aborted) return;

        try {
          const next = await api.run(id, abort.signal);
          if (abort.signal.aborted || current !== events) return;
          record(next);
          if (next.logs_expired) {
            stopFeed();
            setConnection("done");
            return;
          }
        } catch (error) {
          if (abort.signal.aborted || current !== events) return;
          if (isUnauthorized(error)) {
            unauthenticated();
            return;
          }
          if (error instanceof ApiError && error.status === 404) {
            fail("这条执行记录已不存在。", "missing");
            return;
          }
        }

        if (current.readyState === EventSource.CLOSED) {
          stopFeed();
          openFeed();
          // A server response that closes immediately may not open the stream first.
          recover();
        } else if (current.readyState !== EventSource.OPEN) {
          recover();
        }
      }, 3000);
    }

    function openFeed() {
      const current = api.events(id, buffer.lastSequence);
      events = current;
      current.onopen = () => {
        clearTimeout(recovery);
        recovery = undefined;
        setConnection("open");
      };
      current.onerror = () => {
        setConnection("reconnecting");
        recover();
      };
      current.addEventListener("auth", unauthenticated);
      current.addEventListener("status", recordStatus);
      current.addEventListener("log", (event: MessageEvent<string>) => {
        try {
          appendLog(buffer, JSON.parse(event.data) as LogChunk);
        } catch {
          fail("收到无法读取的日志，请重试。");
          return;
        }
        if (frame === undefined) {
          frame = requestAnimationFrame(() => {
            frame = undefined;
            setRevision((previous) => previous + 1);
          });
        }
      });
      current.addEventListener("done", (event: MessageEvent<string>) => {
        if (!recordStatus(event)) return;
        stopFeed();
        setConnection("done");
      });
    }

    api.run(id, abort.signal).then(
      (next) => {
        if (abort.signal.aborted) return;
        record(next);
        if (next.logs_expired) setConnection("done");
        else {
          setConnection("connecting");
          openFeed();
        }
      },
      (error: unknown) => {
        if (abort.signal.aborted) return;
        if (isUnauthorized(error)) unauthenticated();
        else if (error instanceof ApiError && error.status === 404)
          fail("找不到这条执行记录。", "missing");
        else fail("无法读取执行记录，请重试。");
      },
    );

    return () => {
      abort.abort();
      stopFeed();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, [attempt, buffer, id, onUnauthorized]);

  function retry() {
    setError("");
    setConnection("loading");
    setAttempt((previous) => previous + 1);
  }

  return { run, buffer, connection, error, revision, retry };
}
