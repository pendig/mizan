import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { useRegisterMutation } from '@/features/api/mizanApi';
import { DataCard } from '@/components/DataRow';

export function RegisterPage() {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [register, { isLoading }] = useRegisterMutation();
  const navigate = useNavigate();

  return (
    <div className="mx-auto grid w-full max-w-md gap-4 pt-10">
      <DataCard title="Daftar akun">
        <form
          className="grid gap-3"
          onSubmit={async (event) => {
            event.preventDefault();
            await register({ email: email.trim(), password }).unwrap();
            navigate('/login');
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
            {isLoading ? 'Mendaftar...' : 'Register'}
          </button>
        </form>
      </DataCard>
      <p className="text-xs text-slate-400">
        Sudah punya akun? <Link to="/login" className="text-shell-accent">Login</Link>
      </p>
    </div>
  );
}
