import type { ReactNode } from 'react';

export function DataCard({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-2xl border border-shell-border bg-shell-card p-4 shadow-sm">
      <header className="mb-3 flex items-start justify-between gap-4">
        <h2 className="text-sm font-semibold uppercase tracking-[0.2em] text-slate-200">{title}</h2>
        {action}
      </header>
      <div>{children}</div>
    </section>
  );
}

export function EmptyState({ children }: { children: ReactNode }) {
  return <div className="rounded-xl border border-dashed border-shell-border p-3 text-slate-400">{children}</div>;
}
