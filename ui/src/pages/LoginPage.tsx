import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { useLoginMutation } from '@/features/api/mizanApi';
import { useAppSelector } from '@/features/hooks';
import { selectAuth } from '@/features/auth/authSlice';
import { DataCard } from '@/components/DataRow';

export function LoginPage() {
  const auth = useAppSelector(selectAuth);
  const navigate = useNavigate();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [login, { isLoading, error }] = useLoginMutation();

  useEffect(() => {
    if (!auth.token) {
      return;
    }
    navigate(auth.role === 'admin' ? '/admin' : '/user', { replace: true });
  }, [auth.role, auth.token, navigate]);

  return (
    <div className="mx-auto grid w-full max-w-md gap-4 pt-10">
      <DataCard title="Login">
        <form
          className="grid gap-3"
          onSubmit={async (event) => {
            event.preventDefault();
            await login({ email: email.trim(), password }).unwrap();
          }}
        >
          <label className="grid gap-1 text-sm">
            Email
            <input
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
              type="email"
              className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            />
          </label>
          <label className="grid gap-1 text-sm">
            Password
            <input
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              type="password"
              className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            />
          </label>
          <button
            type="submit"
            disabled={isLoading}
            className="mt-1 rounded-xl bg-shell-accent px-4 py-2 font-semibold text-slate-950 disabled:opacity-50"
          >
            {isLoading ? 'Login...' : 'Masuk'}
          </button>
          {error ? <p className="text-sm text-rose-300">Login gagal, cek email/password.</p> : null}
        </form>
      </DataCard>
      <p className="text-xs text-slate-400">
        Belum punya akun? <Link to="/register" className="text-shell-accent">Register</Link>
      </p>
    </div>
  );
}
