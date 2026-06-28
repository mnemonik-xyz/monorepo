import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import Analytics from "./pages/Analytics";
import Blog from "./pages/Blog";
import BlogPost from "./pages/BlogPost";
import Chat from "./pages/Chat";
import Consent from "./pages/Consent";
import Install from "./pages/Install";
import Landing from "./pages/Landing";
import Ledger from "./pages/Ledger";
import Privacy from "./pages/Privacy";
import Roadmap from "./pages/Roadmap";
import Sign from "./pages/Sign";

/**
 * App router root. Replaces the prior `useState<View>` toggle (see git history
 * for the legacy state machine). Explicit routes plus a catch-all that sends
 * unknown URLs to the landing page. The Evidence Ledger pages (/ledger,
 * /analytics, /blog) are public, crawlable surfaces.
 */
function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Landing />} />
        <Route path="/ledger" element={<Ledger />} />
        <Route path="/analytics" element={<Analytics />} />
        <Route path="/blog" element={<Blog />} />
        <Route path="/blog/:slug" element={<BlogPost />} />
        <Route path="/install" element={<Install />} />
        <Route path="/roadmap" element={<Roadmap />} />
        <Route path="/privacy" element={<Privacy />} />
        <Route path="/chat" element={<Chat />} />
        <Route path="/sign/:correlationId" element={<Sign />} />
        <Route path="/oauth/consent" element={<Consent />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}

export default App;
