import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import Shell from "./shell/Shell";
import Fleet from "./pages/Fleet";
import Printer from "./pages/Printer";
import Jobs from "./pages/Jobs";
import Library from "./pages/Library";
import Projects from "./pages/Projects";
import Filament from "./pages/Filament";
import Service from "./pages/Service";
import Tools from "./pages/Tools";
import Settings from "./pages/Settings";
import Attention from "./pages/Attention";
import StartPrint from "./pages/StartPrint";
import SetupWizard from "./setup/SetupWizard";
import DevComponents from "./pages/DevComponents";
import AuthGate from "./auth/AuthGate";
import DesktopRoot from "./desktop/DesktopRoot";
import { IS_TAURI } from "./shell/TopBar";

export default function App() {
  const app = (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Shell />}>
          {/* Default route redirects to Fleet */}
          <Route index element={<Navigate to="/fleet" replace />} />
          <Route path="fleet/*" element={<Fleet />} />
          <Route path="printer/:id" element={<Printer />} />
          <Route path="jobs/*" element={<Jobs />} />
          <Route path="library/*" element={<Library />} />
          <Route path="projects/*" element={<Projects />} />
          <Route path="filament/*" element={<Filament />} />
          <Route path="service/*" element={<Service />} />
          <Route path="tools/*" element={<Tools />} />
          <Route path="settings/*" element={<Settings />} />
          <Route path="attention" element={<Attention />} />
          <Route path="start" element={<StartPrint />} />
          <Route path="setup" element={<SetupWizard />} />
          {/* Dev-only: not in the rail, reachable by URL */}
          <Route path="dev/components" element={<DevComponents />} />
          {/* 404 fallback within the shell */}
          <Route path="*" element={<NotFound />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
  // The desktop app picks a server; the browser talks to the server that served it.
  return IS_TAURI ? <DesktopRoot>{app}</DesktopRoot> : <AuthGate>{app}</AuthGate>;
}

function NotFound() {
  return (
    <div>
      <div className="page-header">
        <p className="page-header__eyebrow">404</p>
        <h1 className="page-header__title">Page not found</h1>
      </div>
      <p className="page-placeholder">
        The URL you requested does not exist. Use the navigation rail to find
        what you need.
      </p>
    </div>
  );
}
