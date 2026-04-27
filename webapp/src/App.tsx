import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import Chat from "./pages/Chat";
import Install from "./pages/Install";
import Landing from "./pages/Landing";
import Sign from "./pages/Sign";

/**
 * App router root. Replaces the prior `useState<View>` toggle (see git history
 * for the legacy state machine). Four explicit routes plus a catch-all that
 * sends unknown URLs to the landing page.
 */
function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Landing />} />
        <Route path="/install" element={<Install />} />
        <Route path="/chat" element={<Chat />} />
        <Route path="/sign/:correlationId" element={<Sign />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}

export default App;
