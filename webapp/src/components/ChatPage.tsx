import { useRef, useEffect, useCallback, useState } from "react";
import { sendChatMessage, ChatApiError } from "../lib/api";
import type { Message } from "../App";

const SESSION_MESSAGE_LIMIT = 50;

function generateMessageId(): string {
  return crypto.randomUUID();
}

/** Detects hashes, tx IDs, and public keys (32+ hex chars or base58-like strings). */
function containsTechnicalContent(text: string): boolean {
  return /[A-Fa-f0-9]{32,}|[1-9A-HJ-NP-Za-km-z]{32,}/.test(text);
}

function MessageContent({ content }: { content: string }) {
  const parts = content.split(/(`[^`]+`)/g);
  const hasCode = parts.length > 1;

  if (!hasCode && containsTechnicalContent(content)) {
    return <span className="font-mono text-sm">{content}</span>;
  }

  return (
    <>
      {parts.map((part, i) => {
        if (part.startsWith("`") && part.endsWith("`")) {
          const code = part.slice(1, -1);
          return (
            <code
              key={i}
              className="rounded bg-white/10 px-1 py-0.5 font-mono text-sm text-accent-primary"
            >
              {code}
            </code>
          );
        }
        return <span key={i}>{part}</span>;
      })}
    </>
  );
}

interface ChatPageProps {
  onBack: () => void;
  messages: Message[];
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>;
  messageCount: number;
  setMessageCount: React.Dispatch<React.SetStateAction<number>>;
  sessionId: string;
}

export default function ChatPage({
  onBack,
  messages,
  setMessages,
  messageCount,
  setMessageCount,
  sessionId,
}: ChatPageProps) {
  const [input, setInput] = useState("");
  const [isLoading, setIsLoading] = useState(false);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const isLimitReached = messageCount >= SESSION_MESSAGE_LIMIT;

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleSend = useCallback(async () => {
    const trimmed = input.trim();
    if (!trimmed || isLoading || isLimitReached) return;

    const userMessage: Message = {
      id: generateMessageId(),
      role: "user",
      content: trimmed,
    };

    setMessages((prev) => [...prev, userMessage]);
    setInput("");
    setIsLoading(true);

    try {
      const response = await sendChatMessage({
        message: trimmed,
        session_id: sessionId,
      });

      const botMessage: Message = {
        id: generateMessageId(),
        role: "bot",
        content: response.response,
      };

      setMessages((prev) => [...prev, botMessage]);
      setMessageCount((prev) => prev + 1);
    } catch (err) {
      let errorText = "Unexpected error.";
      if (err instanceof ChatApiError) {
        errorText = err.message;
      }

      const errorMessage: Message = {
        id: generateMessageId(),
        role: "bot",
        content: errorText,
        isError: true,
      };

      setMessages((prev) => [...prev, errorMessage]);
      setMessageCount((prev) => prev + 1);
    } finally {
      setIsLoading(false);
    }
  }, [
    input,
    isLoading,
    isLimitReached,
    sessionId,
    setMessages,
    setMessageCount,
  ]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend]
  );

  return (
    <div className="flex h-screen flex-col bg-background">
      <header className="flex shrink-0 items-center justify-between border-b border-text-muted/20 px-6 py-4">
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={onBack}
            className="text-text-muted transition-colors hover:text-text-primary"
            aria-label="Back to landing page"
          >
            <svg
              width="20"
              height="20"
              viewBox="0 0 20 20"
              fill="none"
              aria-hidden="true"
            >
              <path
                d="M12.5 15L7.5 10L12.5 5"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
          <h1 className="text-lg font-semibold text-text-primary">
            Protocol Chat
          </h1>
        </div>
        <span
          className="text-sm text-text-muted"
          aria-label="Session message counter"
        >
          {messageCount}/{SESSION_MESSAGE_LIMIT}
        </span>
      </header>

      <div
        className="flex-1 overflow-y-auto px-4 py-6"
        role="log"
        aria-live="polite"
      >
        <div className="mx-auto max-w-3xl space-y-4">
          {messages.length === 0 && (
            <div className="py-16 text-center">
              <p className="text-text-muted">
                Ask a question about the Mnemonic Protocol.
              </p>
            </div>
          )}

          {messages.map((msg) => (
            <div
              key={msg.id}
              className={`flex ${
                msg.role === "user" ? "justify-end" : "justify-start"
              }`}
            >
              <div
                className={`max-w-[80%] rounded-lg px-4 py-3 text-sm leading-relaxed ${
                  msg.role === "user"
                    ? "bg-accent-primary/15 text-text-primary"
                    : msg.isError
                    ? "border border-error/30 bg-error/10 text-error"
                    : "bg-white/5 text-text-primary"
                }`}
              >
                {msg.isError ? (
                  <span>{msg.content}</span>
                ) : (
                  <MessageContent content={msg.content} />
                )}
              </div>
            </div>
          ))}

          {isLoading && (
            <div className="flex justify-start">
              <div className="rounded-lg bg-white/5 px-4 py-3 text-sm text-text-muted">
                Processing...
              </div>
            </div>
          )}

          <div ref={messagesEndRef} />
        </div>
      </div>

      {isLimitReached && (
        <div
          className="border-t border-error/30 bg-error/10 px-6 py-3 text-center text-sm text-error"
          role="alert"
        >
          Session limit reached. Start a new session to continue.
        </div>
      )}

      <div className="shrink-0 border-t border-text-muted/20 px-4 py-4">
        <div className="mx-auto flex max-w-3xl gap-3">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={isLoading || isLimitReached}
            placeholder={
              isLimitReached
                ? "Session limit reached."
                : "Ask about the Mnemonic Protocol..."
            }
            rows={1}
            className="flex-1 resize-none rounded-lg border border-text-muted/20 bg-white/5 px-4 py-3 text-sm text-text-primary placeholder-text-muted/50 outline-none transition-colors focus:border-accent-primary/50 disabled:cursor-not-allowed disabled:opacity-50"
            aria-label="Message input"
          />
          <button
            onClick={handleSend}
            disabled={isLoading || isLimitReached || !input.trim()}
            className="shrink-0 rounded-lg bg-accent-primary px-5 py-3 text-sm font-medium text-background transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
            aria-label="Send message"
          >
            Send
          </button>
        </div>
      </div>
    </div>
  );
}
