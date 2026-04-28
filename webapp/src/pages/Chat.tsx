import { useState, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import ChatPage from "../components/ChatPage";
import LandingPage from "../components/LandingPage";
import type { Message } from "../types";

/**
 * `/chat` route — preserves the previous root-route demo behavior verbatim.
 *
 * Owns the `messages` / `messageCount` / `sessionId` state that App.tsx used
 * to hold so the route is self-contained. The "back" arrow inside ChatPage
 * navigates to `/` (Landing) via `useNavigate`.
 */
function generateSessionId(): string {
  return crypto.randomUUID();
}

type View = "input" | "chat";

export default function Chat() {
  const navigate = useNavigate();

  const [view, setView] = useState<View>("input");
  const [messages, setMessages] = useState<Message[]>([]);
  const [messageCount, setMessageCount] = useState(0);
  const [sessionId] = useState(generateSessionId);

  const handleFirstMessage = useCallback(() => {
    setView("chat");
  }, []);

  return view === "chat" ? (
    <ChatPage
      onBack={() => navigate("/")}
      messages={messages}
      setMessages={setMessages}
      messageCount={messageCount}
      setMessageCount={setMessageCount}
      sessionId={sessionId}
    />
  ) : (
    <LandingPage
      onStartChat={handleFirstMessage}
      messages={messages}
      setMessages={setMessages}
      messageCount={messageCount}
      setMessageCount={setMessageCount}
      sessionId={sessionId}
      onNavigateToChat={() => setView("chat")}
    />
  );
}
