import { Navigate, Route, Routes } from 'react-router-dom';
import { LoginPage } from '@/pages/LoginPage';
import { RegisterPage } from '@/pages/RegisterPage';
import { UserDashboardPage } from '@/pages/user/UserDashboardPage';
import { UserKeysPage } from '@/pages/user/UserKeysPage';
import { UserUsagePage } from '@/pages/user/UserUsagePage';
import { AdminOverviewPage } from '@/pages/admin/AdminOverviewPage';
import { AdminProvidersPage } from '@/pages/admin/AdminProvidersPage';
import { AdminRoutesPage } from '@/pages/admin/AdminRoutesPage';
import { AdminUsagePage } from '@/pages/admin/AdminUsagePage';
import { AdminDaemonNodesPage } from '@/pages/admin/AdminDaemonNodesPage';
import { NotFoundPage } from '@/pages/NotFoundPage';
import { RequireAuth } from '@/components/RequireAuth';
import Shell from '@/components/Shell';

export function App() {
  return (
    <Shell>
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route path="/register" element={<RegisterPage />} />

        <Route
          path="/admin"
          element={
            <RequireAuth requireAdmin>
              <AdminOverviewPage />
            </RequireAuth>
          }
        />
        <Route
          path="/admin/providers"
          element={
            <RequireAuth requireAdmin>
              <AdminProvidersPage />
            </RequireAuth>
          }
        />
        <Route
          path="/admin/routes"
          element={
            <RequireAuth requireAdmin>
              <AdminRoutesPage />
            </RequireAuth>
          }
        />
        <Route
          path="/admin/usage"
          element={
            <RequireAuth requireAdmin>
              <AdminUsagePage />
            </RequireAuth>
          }
        />

        <Route
          path="/admin/daemons"
          element={
            <RequireAuth requireAdmin>
              <AdminDaemonNodesPage />
            </RequireAuth>
          }
        />

        <Route
          path="/user"
          element={
            <RequireAuth>
              <UserDashboardPage />
            </RequireAuth>
          }
        />
        <Route
          path="/user/keys"
          element={
            <RequireAuth>
              <UserKeysPage />
            </RequireAuth>
          }
        />
        <Route
          path="/user/usage"
          element={
            <RequireAuth>
              <UserUsagePage />
            </RequireAuth>
          }
        />

        <Route path="/" element={<Navigate to="/user" replace />} />
        <Route path="*" element={<NotFoundPage />} />
      </Routes>
    </Shell>
  );
}
