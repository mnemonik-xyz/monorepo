import { useState } from "react";
import LandingPage from "./components/LandingPage";
import ChatPage from "./components/ChatPage";

type View = "landing" | "chat";

function App() {
  const [view, setView] = useState<View>("landing");

  if (view === "chat") {
    return <ChatPage onBack={() => setView("landing")} />;
  }

  return <LandingPage onStartChat={() => setView("chat")} />;
}

export default App;
