import { useState, useCallback, useRef, useEffect } from "react";
import {
  sendChatMessage,
  ChatApiError,
  fetchPublicStats,
  MCP_BASE,
  type PublicStats,
} from "../lib/api";
import type { Message } from "../types";

const SESSION_MESSAGE_LIMIT = 50;

function generateMessageId(): string {
  return crypto.randomUUID();
}

interface LandingPageProps {
  onStartChat: () => void;
  messages: Message[];
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>;
  messageCount: number;
  setMessageCount: React.Dispatch<React.SetStateAction<number>>;
  sessionId: string;
  onNavigateToChat: () => void;
}

function LandingPage({
  onStartChat,
  messages,
  setMessages,
  messageCount,
  setMessageCount,
  sessionId,
  onNavigateToChat,
}: LandingPageProps) {
  const [input, setInput] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [stats, setStats] = useState<PublicStats | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const isLimitReached = messageCount >= SESSION_MESSAGE_LIMIT;
  const hasMessages = messages.length > 0;

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    let cancelled = false;
    fetchPublicStats().then((s) => {
      if (!cancelled) setStats(s);
    });
    return () => {
      cancelled = true;
    };
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

    // Navigate to chat view immediately on first message
    if (!hasMessages) {
      onStartChat();
    }

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
    hasMessages,
    onStartChat,
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
    [handleSend],
  );

  return (
    <main className="flex min-h-screen flex-col items-center justify-center px-4 py-12">
      <div className="w-full max-w-2xl space-y-10 text-center">
        <header>
          <h1 className="text-4xl font-bold tracking-tight text-accent-primary sm:text-5xl">
            Mnemonic Protocol
          </h1>
          <p className="mt-3 text-lg text-text-muted">
            Verifiable memory infrastructure for AI agents.
          </p>
        </header>

        {stats && (
          <section
            aria-label="Network traction"
            className="grid grid-cols-3 gap-4 rounded-lg border border-text-muted/20 bg-white/5 px-4 py-5"
          >
            <StatCell value={stats.unique_users} label="unique users" />
            <StatCell value={stats.saved_on_node} label="memories on node" />
            <StatCell value={stats.saved_onchain} label="memories on-chain" />
          </section>
        )}

        <section
          className="space-y-6 text-left"
          aria-label="Protocol description"
        >
          <p className="leading-relaxed text-text-primary">
            AI agents increasingly operate across long-running tasks, tools,
            sessions, and providers, but their memory remains fragile. Context
            windows are temporary, vendor-native memory is hard to audit, and
            conventional vector stores provide persistence without cryptographic
            provenance.
          </p>

          <p className="leading-relaxed text-text-primary">
            Mnemonic Protocol introduces a verifiable memory layer for AI
            agents. It treats memory as a portable, signed artifact rather than
            an opaque database row: something an agent can{" "}
            <span className="font-mono text-accent-primary">recall</span>, carry
            across systems, and prove has not been silently changed.
          </p>

          <p className="leading-relaxed text-text-muted italic">
            Trustless agents cannot work without trustless agentic memory.
          </p>
        </section>

        {/* Chat input on landing page */}
        <div className="mx-auto w-full max-w-2xl">
          <div className="flex gap-3">
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              disabled={isLoading || isLimitReached}
              placeholder="Ask about the Mnemonic Protocol..."
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
              {isLoading ? "..." : "Ask"}
            </button>
          </div>
          {hasMessages && (
            <button
              type="button"
              onClick={onNavigateToChat}
              className="mt-3 text-sm text-accent-primary transition-opacity hover:opacity-80"
            >
              Continue chat ({messages.length} messages)
            </button>
          )}
        </div>

        <nav className="flex flex-col items-center gap-4 sm:flex-row sm:justify-center">
          <a
            href={`${MCP_BASE}/download-knowledge`}
            download
            className="rounded-md border border-text-muted/30 px-6 py-3 text-sm font-semibold text-text-primary transition-colors hover:border-accent-secondary hover:text-accent-secondary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-secondary"
          >
            Download protocol knowledge
          </a>
        </nav>
      </div>
    </main>
  );
}

function StatCell({ value, label }: { value: number; label: string }) {
  return (
    <div className="flex flex-col items-center">
      <span className="font-mono text-2xl font-semibold tabular-nums text-accent-primary sm:text-3xl">
        {value.toLocaleString()}
      </span>
      <span className="mt-1 text-xs uppercase tracking-wider text-text-muted">
        {label}
      </span>
    </div>
  );
}

export default LandingPage;
