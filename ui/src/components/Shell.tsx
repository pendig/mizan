import { Link, useNavigate } from 'react-router-dom';
import { useAppDispatch, useAppSelector } from '@/features/hooks';
import { useLogoutMutation } from '@/features/api/mizanApi';
import { clearSession, selectAuth } from '@/features/auth/authSlice';
import { useState } from 'react';

export default function Shell({ children }: { children: React.ReactNode }) {
  const auth = useAppSelector(selectAuth);
  const [logout] = useLogoutMutation();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);

  const isAdmin = auth.role === 'admin';

  const onLogout = async () => {
    setBusy(true);
    try {
      await logout().unwrap();
    } finally {
      dispatch(clearSession());
      setBusy(false);
      navigate('/login');
    }
  };

  return (
    <div className="min-h-screen bg-shell-bg text-slate-100">
      <header className="border-b border-shell-border/80 bg-shell-card/80 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-4 py-4">
          <div className="font-semibold tracking-tight">
            <Link to="/" className="hover:text-shell-accent">
              Mizan
            </Link>
          </div>
          <nav className="flex flex-wrap items-center gap-2 text-sm">
            {!auth.token ? (
              <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/login">
                Login
              </Link>
            ) : (
              <>
                {isAdmin ? (
                  <>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/admin">
                      Admin
                    </Link>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/admin/providers">
                      Providers
                    </Link>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/admin/routes">
                      Routes
                    </Link>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/admin/usage">
                      Usage
                    </Link>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/admin/daemons">
                      Daemons
                    </Link>
                  </>
                ) : (
                  <>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/user">
                      Dashboard
                    </Link>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/user/keys">
                      API Keys
                    </Link>
                    <Link className="rounded-xl px-2 py-1 hover:text-shell-accent" to="/user/usage">
                      Usage
                    </Link>
                  </>
                )}
                <button
                  type="button"
                  onClick={onLogout}
                  disabled={busy}
                  className="rounded-xl border border-shell-border px-2 py-1 hover:text-white disabled:opacity-50"
                >
                  {busy ? 'Keluar...' : 'Keluar'}
                </button>
              </>
            )}
          </nav>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-4 py-6">{children}</main>
    </div>
  );
}
