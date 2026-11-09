import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import Shell from "./shell/Shell";
import Fleet from "./pages/Fleet";
import Jobs from "./pages/Jobs";
import Library from "./pages/Library";
import Projects from "./pages/Projects";
import Filament from "./pages/Filament";
import Service from "./pages/Service";
import Tools from "./pages/Tools";
import Settings from "./pages/Settings";
import DevComponents from "./pages/DevComponents";

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Shell />}>
          {/* Default route redirects to Fleet */}
          <Route index element={<Navigate to="/fleet" replace />} />
          <Route path="fleet/*" element={<Fleet />} />
          <Route path="jobs/*" element={<Jobs />} />
          <Route path="library/*" element={<Library />} />
          <Route path="projects/*" element={<Projects />} />
          <Route path="filament/*" element={<Filament />} />
          <Route path="service/*" element={<Service />} />
          <Route path="tools/*" element={<Tools />} />
          <Route path="settings/*" element={<Settings />} />
          {/* Dev-only: not in the rail, reachable by URL */}
          <Route path="dev/components" element={<DevComponents />} />
          {/* 404 fallback within the shell */}
          <Route path="*" element={<NotFound />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
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
