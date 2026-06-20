import { Navigate } from 'react-router-dom';
import { useAppSelector } from '@/features/hooks';
import { selectAuth } from '@/features/auth/authSlice';

export function RequireAuth({
  children,
  requireAdmin,
}: {
  children: React.ReactNode;
  requireAdmin?: boolean;
}) {
  const auth = useAppSelector(selectAuth);

  if (!auth.token) {
    return <Navigate to="/login" replace />;
  }

  if (requireAdmin && auth.role !== 'admin') {
    return <Navigate to="/user" replace />;
  }

  return <>{children}</>;
}
