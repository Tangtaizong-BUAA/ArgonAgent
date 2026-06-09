export const TOOL_MARKUP_START_PATTERNS: RegExp[] = [
  /<[^>\n]{0,80}dsml[^>\n]{0,120}(tool_calls|invoke|parameter)[^>\n]*>/i,
  /<\s*tool_call\b[^>\n]*>/i,
  /<\s*invoke\b[^>\n]*>/i,
  /\{\s*"tool_calls"\s*:/i,
];

export const TOOL_MARKUP_END_PATTERNS: RegExp[] = [
  /<\/[^>\n]{0,80}dsml[^>\n]{0,120}(tool_calls|invoke|parameter)\s*>/i,
  /<\/\s*tool_call\s*>/i,
  /<\/\s*invoke\s*>/i,
];

const TOOL_MARKUP_PREFIX_MARKERS = [
  "<｜｜DSML｜｜tool_calls",
  "<｜｜DSML｜｜invoke",
  "<｜｜DSML｜｜parameter",
  "<tool_call",
  "<invoke",
  '{"tool_calls"',
];

function decodeEscapedWhitespace(value: string): string {
  if (!value) {
    return "";
  }
  let output = value;
  for (let index = 0; index < 4; index += 1) {
    const next = output
      .replace(/\\\\r\\\\n/g, "\n")
      .replace(/\\\\n/g, "\n")
      .replace(/\\\\t/g, "\t")
      .replace(/\\r\\n/g, "\n")
      .replace(/\\n/g, "\n")
      .replace(/\\t/g, "\t");
    if (next === output) {
      break;
    }
    output = next;
  }
  return output.replace(/\\+\n/g, "\n");
}

export function normalizeStreamChunk(chunk: string): string {
  if (!chunk) {
    return "";
  }
  return decodeEscapedWhitespace(chunk).replace(/\r\n/g, "\n");
}

export function findFirstPattern(text: string, patterns: RegExp[]): { index: number; value: string } | null {
  let hit: { index: number; value: string } | null = null;
  for (const pattern of patterns) {
    const match = pattern.exec(text);
    if (!match || match.index < 0) {
      continue;
    }
    if (!hit || match.index < hit.index) {
      hit = { index: match.index, value: match[0] };
    }
  }
  return hit;
}

function findBalancedJsonObjectEnd(text: string, startIndex: number): number | null {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = startIndex; index < text.length; index += 1) {
    const char = text[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === '"') {
        inString = false;
      }
      continue;
    }
    if (char === '"') {
      inString = true;
      continue;
    }
    if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return index + 1;
      }
    }
  }
  return null;
}

export function stripLeadingToolCallsJson(text: string): { status: "none" | "complete" | "incomplete"; text: string } {
  const match = /^\s*\{\s*"tool_calls"\s*:/i.exec(text);
  if (!match) {
    return { status: "none", text };
  }
  const objectStart = text.indexOf("{");
  const objectEnd = findBalancedJsonObjectEnd(text, objectStart);
  if (objectEnd === null) {
    return { status: "incomplete", text };
  }
  return { status: "complete", text: text.slice(objectEnd).replace(/^\s*\n?/, "") };
}

function stripToolCallsJsonSegments(text: string): string {
  let output = "";
  let cursor = 0;
  while (cursor < text.length) {
    const match = /\{\s*"tool_calls"\s*:/i.exec(text.slice(cursor));
    if (!match || match.index < 0) {
      output += text.slice(cursor);
      break;
    }
    const start = cursor + match.index;
    output += text.slice(cursor, start);
    const end = findBalancedJsonObjectEnd(text, start);
    if (end === null) {
      output += text.slice(start);
      break;
    }
    cursor = end;
  }
  return output;
}

export function trimLeadingBlankLines(value: string): string {
  return value.replace(/^\s*\n+/, "");
}

export function extractTrailingPotentialMarkup(text: string): { stable: string; trailing: string } {
  if (!text) {
    return { stable: "", trailing: "" };
  }
  for (let length = Math.min(80, text.length); length >= 1; length -= 1) {
    const suffix = text.slice(-length);
    if (TOOL_MARKUP_PREFIX_MARKERS.some((marker) => marker.startsWith(suffix))) {
      return {
        stable: text.slice(0, -length),
        trailing: suffix,
      };
    }
  }
  return { stable: text, trailing: "" };
}

export function repairPseudoMarkdownNewlines(text: string): string {
  if (!text) {
    return "";
  }
  let output = text;
  output = output.replace(/(```[A-Za-z0-9_+.-]*)n(?=\S)/g, "$1\n");
  output = output.replace(/\b(import|from|def|class|return|assert|with|for|if|elif|else|try|except|finally)n(?=[A-Za-z_#@])/g, "$1\n");
  output = output.replace(/([。！？.!?；;：:）\]\}])nn(?=\S)/g, "$1\n\n");
  output = output.replace(/([。！？.!?；;：:）\]\}])n(?=(#{1,6}\s|---|[-*]\s|```|\d+\.\s))/g, "$1\n");
  output = output.replace(
    /(^|[^A-Za-z0-9_])nn(?=(#{1,6}\s|---|[-*]\s|```|\|(?:[^|]*\|)+|\d+\.\s))/g,
    "$1\n\n",
  );
  output = output.replace(/nn(?=\n)/g, "\n");
  output = output.replace(/\n{3,}/g, "\n\n");
  return output;
}

export function stripLeakedToolMarkup(content: string): string {
  if (!content) {
    return "";
  }
  let output = normalizeStreamChunk(content);
  output = output.replace(
    /<[^>\n]{0,120}dsml[^>\n]{0,220}tool_calls[^>]*>[\s\S]*?<\/[^>\n]{0,120}dsml[^>\n]{0,220}tool_calls\s*>/gi,
    "",
  );
  output = output.replace(
    /<\/?[^>\n]{0,120}dsml[^>\n]{0,220}(invoke|parameter|tool_calls)[^>]*>/gi,
    "",
  );
  output = stripToolCallsJsonSegments(output);
  output = output
    .split("\n")
    .filter((line) => {
      const lowered = line.toLowerCase();
      if (
        /(?:^|\s)n?=\"?(?:file_read|file\.read|read|list_directory|file\.list_directory|search|bash|execute_command)\"?>/i.test(
          line,
        )
      ) {
        return false;
      }
      if (
        lowered.includes("string=\"true\"") &&
        (lowered.includes("list_directory") ||
          lowered.includes("file_read") ||
          lowered.includes("tool_calls") ||
          lowered.includes("parameter"))
      ) {
        return false;
      }
      if (
        lowered.includes(".researchcode/argon_agent/workspaces/projects/") &&
        (lowered.includes("list_directory") || lowered.includes("file_read") || lowered.includes("tool_calls"))
      ) {
        return false;
      }
      if (!lowered.includes("dsml")) {
        return true;
      }
      return !(
        lowered.includes("tool_calls") ||
        lowered.includes("invoke name") ||
        lowered.includes("parameter name")
      );
    })
    .join("\n");
  return output.replace(/\n{3,}/g, "\n\n").trim();
}

export function resolveFinalStreamingContent(streamed: string, finalContent: string): string {
  const streamedText = repairPseudoMarkdownNewlines(normalizeStreamChunk(streamed)).trim();
  const finalText = repairPseudoMarkdownNewlines(normalizeStreamChunk(finalContent)).trim();
  if (!streamedText) {
    return finalText;
  }
  if (!finalText) {
    return streamedText;
  }
  if (streamedText.includes(finalText)) {
    return streamedText;
  }
  if (finalText.includes(streamedText)) {
    return finalText;
  }
  const shorter = streamedText.length <= finalText.length ? streamedText : finalText;
  const longer = streamedText.length > finalText.length ? streamedText : finalText;
  if (shorter.length > 40 && longer.includes(shorter.slice(0, Math.min(shorter.length, 120)))) {
    return longer;
  }
  if (finalText.length >= streamedText.length * 0.8) {
    return finalText;
  }
  return `${streamedText}\n\n${finalText}`;
}
