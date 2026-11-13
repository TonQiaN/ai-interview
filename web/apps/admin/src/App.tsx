import { Navigate, Route, Routes } from "react-router";
import { Layout } from "./layout/Layout";
import { CandidateDetailPage } from "./pages/CandidateDetail";
import { CandidatesPage } from "./pages/Candidates";
import { InterviewDetailPage } from "./pages/InterviewDetail";
import { InterviewNewPage } from "./pages/InterviewNew";
import { InterviewsPage } from "./pages/Interviews";
import { LoginPage } from "./pages/Login";
import { NotFoundPage } from "./pages/NotFound";
import { OverviewPage } from "./pages/Overview";
import { PositionDetailPage } from "./pages/PositionDetail";
import { PositionsPage } from "./pages/Positions";
import { SettingsPage } from "./pages/Settings";
import { TemplateEditPage } from "./pages/TemplateEdit";
import { TemplatesPage } from "./pages/Templates";
import { VoicesPage } from "./pages/Voices";

export function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route element={<Layout />}>
        <Route index element={<OverviewPage />} />
        <Route path="interviews" element={<InterviewsPage />} />
        <Route path="interviews/new" element={<InterviewNewPage />} />
        <Route path="interviews/:id" element={<InterviewDetailPage />} />
        <Route path="candidates" element={<CandidatesPage />} />
        <Route path="candidates/:id" element={<CandidateDetailPage />} />
        <Route path="positions" element={<PositionsPage />} />
        <Route path="positions/:id" element={<PositionDetailPage />} />
        <Route path="templates" element={<TemplatesPage />} />
        <Route path="templates/new" element={<TemplateEditPage />} />
        <Route path="templates/:id" element={<TemplateEditPage />} />
        <Route path="voices" element={<VoicesPage />} />
        <Route path="settings" element={<SettingsPage />} />
        <Route path="home" element={<Navigate to="/" replace />} />
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  );
}
