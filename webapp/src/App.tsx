import { useState, useCallback } from "react";
import LandingPage from "./components/LandingPage";
import ChatPage from "./components/ChatPage";

export interface Message {
  id: string;
  role: "user" | "bot";
  content: string;
  isError?: boolean;
}

type View = "landing" | "chat";

function generateSessionId(): string {
  return crypto.randomUUID();
}

function App() {
  const [view, setView] = useState<View>("landing");
  const [messages, setMessages] = useState<Message[]>([]);
  const [messageCount, setMessageCount] = useState(0);
  const [sessionId] = useState(generateSessionId);

  const handleFirstMessage = useCallback(() => {
    setView("chat");
  }, []);

  return view === "chat" ? (
    <ChatPage
      onBack={() => setView("landing")}
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

export default App;
