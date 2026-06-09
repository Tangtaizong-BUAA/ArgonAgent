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
  return output.replace(/\\+\n/g, "\n").replace(/\r\n/g, "\n");
}

function repairPseudoMarkdownNewlines(text: string): string {
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
  return output.replace(/\n{3,}/g, "\n\n");
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

export function normalizeAgentTextForDuplicateComparison(value: string): string {
  let output = decodeEscapedWhitespace(value);
  output = output.replace(
    /<[^>\n]{0,120}dsml[^>\n]{0,220}tool_calls[^>]*>[\s\S]*?<\/[^>\n]{0,120}dsml[^>\n]{0,220}tool_calls\s*>/gi,
    "",
  );
  output = output.replace(
    /<\/?[^>\n]{0,120}dsml[^>\n]{0,220}(invoke|parameter|tool_calls)[^>]*>/gi,
    "",
  );
  output = output.replace(/<\/?parameter[^>\n]{0,200}>/gi, "");
  output = output.replace(/<\/?invoke[^>\n]{0,200}>/gi, "");
  output = stripToolCallsJsonSegments(output);
  output = repairPseudoMarkdownNewlines(output);
  return output.replace(/\s+/g, " ").trim();
}

export function isDuplicateAgentText(previousContent: string | undefined, nextContent: string): boolean {
  const previous = normalizeAgentTextForDuplicateComparison(previousContent ?? "");
  const next = normalizeAgentTextForDuplicateComparison(nextContent);
  return Boolean(previous && next && previous === next);
}
