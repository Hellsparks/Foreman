/** Projects (task 5.1, W9): list, templates, new project, project detail. */
import { Route, Routes } from "react-router-dom";
import ProjectList from "../projects/ProjectList";
import TemplateEditor, { TemplateList } from "../projects/TemplateEditor";
import ProjectNew from "../projects/ProjectNew";
import ProjectDetail from "../projects/ProjectDetail";
import "../projects/projects.css";

export default function Projects() {
  return (
    <Routes>
      <Route index element={<ProjectList />} />
      <Route path="templates" element={<TemplateList />} />
      <Route path="templates/new" element={<TemplateEditor />} />
      <Route path="templates/:id" element={<TemplateEditor />} />
      <Route path="new" element={<ProjectNew />} />
      <Route path=":id" element={<ProjectDetail />} />
    </Routes>
  );
}
