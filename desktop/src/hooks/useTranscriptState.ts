import { useState } from "react";
import type { TranscriptMessage } from "@/types";

export function useTranscriptState() {
  const [messages, setMessages] = useState<TranscriptMessage[]>([]);
  const [inputValue, setInputValue] = useState("");
  const [runTitle, setRunTitle] = useState("新会话");
  const [isStreaming, setIsStreaming] = useState(false);
  const [composerFocusNonce, setComposerFocusNonce] = useState(0);

  return {
    messages,
    setMessages,
    inputValue,
    setInputValue,
    runTitle,
    setRunTitle,
    isStreaming,
    setIsStreaming,
    composerFocusNonce,
    setComposerFocusNonce,
  };
}
