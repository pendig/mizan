import { useState } from 'react';
import { useListAdminUsageQuery, useGrantCreditsMutation } from '@/features/api/mizanApi';
import { DataCard, EmptyState } from '@/components/DataRow';

export function AdminUsagePage() {
  const [userId, setUserId] = useState('');
  const [grantAmount, setGrantAmount] = useState('1000000');
  const [reason, setReason] = useState('manual_adjustment');

  const { data, refetch, isLoading } = useListAdminUsageQuery({
    userId: userId || undefined,
  });

  const [grantCredits, { isLoading: granting }] = useGrantCreditsMutation();

  return (
    <div className="grid gap-4">
      <DataCard title="Manual credit grant" action={
        <button
          type="button"
          onClick={() =>
            grantCredits({
              user_id: userId,
              amount_microcredits: Number(grantAmount || 0),
              reason,
            })
          }
          disabled={!userId || !grantAmount || granting}
          className="rounded-lg border border-shell-border px-3 py-2"
        >
          Grant
        </button>
      }>
        <div className="grid gap-2 sm:grid-cols-3">
          <input
            value={userId}
            onChange={(event) => setUserId(event.target.value)}
            placeholder="User UUID"
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <input
            value={grantAmount}
            onChange={(event) => setGrantAmount(event.target.value)}
            placeholder="Jumlah microcredits"
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <input
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            placeholder="Alasan"
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
        </div>
      </DataCard>

      <DataCard title="Usage list (admin)">
        <button
          className="mb-3 rounded-lg border border-shell-border px-2 py-1 text-xs"
          onClick={() => refetch()}
          type="button"
        >
          Refresh
        </button>
        {isLoading ? (
          <p>Loading...</p>
        ) : data?.data.length === 0 ? (
          <EmptyState>Tidak ada usage.</EmptyState>
        ) : (
          <ul className="space-y-2">
            {data?.data.map((entry) => (
              <li key={entry.id} className="rounded-lg border border-shell-border p-3">
                <p className="font-mono text-sm">{entry.model}</p>
                <p className="text-xs text-slate-400">{entry.created_at}</p>
              </li>
            ))}
          </ul>
        )}
      </DataCard>
    </div>
  );
}
