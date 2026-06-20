import { Link } from 'react-router-dom';

export function NotFoundPage() {
  return (
    <div className="mx-auto mt-10 max-w-xl rounded-2xl border border-shell-border bg-shell-card p-6">
      <h1 className="mb-2 text-lg font-semibold">Page tidak ditemukan</h1>
      <p className="mb-4 text-sm text-slate-400">Coba akses halaman yang ada di menu.</p>
      <Link className="text-shell-accent" to="/">
        Kembali ke beranda
      </Link>
    </div>
  );
}
