import type { LogChunk } from "./model";

export interface LogLine {
  at: number;
  parts: { text: string; stream: LogChunk["stream"] }[];
}

export interface LogBuffer {
  lines: LogLine[];
  nextLine: boolean;
  lastSequence: number | undefined;
}

export function newLogBuffer(): LogBuffer {
  return { lines: [], nextLine: true, lastSequence: undefined };
}

/** Preserve chronological fragments and start a display line only after a newline. */
export function appendLog(buffer: LogBuffer, chunk: LogChunk) {
  if (buffer.lastSequence !== undefined && chunk.seq <= buffer.lastSequence) return;
  buffer.lastSequence = chunk.seq;

  let start = 0;
  while (start < chunk.text.length) {
    const boundary = chunk.text.indexOf("\n", start);
    const end = boundary === -1 ? chunk.text.length : boundary;
    if (buffer.nextLine) buffer.lines.push({ at: chunk.at, parts: [] });
    const line = buffer.lines[buffer.lines.length - 1];
    const text = chunk.text.slice(start, end);
    if (text) {
      const previous = line.parts.at(-1);
      if (previous?.stream === chunk.stream) previous.text += text;
      else line.parts.push({ text, stream: chunk.stream });
    }
    buffer.nextLine = boundary >= 0;
    start = end + 1;
  }
}
