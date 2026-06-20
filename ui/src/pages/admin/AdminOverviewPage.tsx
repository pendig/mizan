import { Link } from 'react-router-dom';
import { DataCard } from '@/components/DataRow';

export function AdminOverviewPage() {
  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      {[
        ['Provider', '/admin/providers'],
        ['Model Routes', '/admin/routes'],
        ['Usage', '/admin/usage'],
        ['Daemons', '/admin/daemons'],
      ].map(([label, href]) => (
        <DataCard key={href} title={label}>
          <Link
            to={href}
            className="inline-flex rounded-xl border border-shell-border px-3 py-2 text-sm hover:border-shell-accent"
          >
            Buka
          </Link>
        </DataCard>
      ))}
    </div>
  );
}
