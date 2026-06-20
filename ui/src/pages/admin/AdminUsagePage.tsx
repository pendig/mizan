import { useMemo, useState } from 'react';
import { useGrantCreditsMutation, useListAdminUsageQuery } from '@/features/api/mizanApi';
import { DataCard, ErrorText, QueryState } from '@/components/DataRow';
import { extractApiErrorMessage } from '@/utils/errorHandling';

type GrantResult = {
  user_id: string;
  balance_microcredits: number;
};

export function AdminUsagePage() {
  const [filters, setFilters] = useState({
    userId: '',
    daemonNodeId: '',
    hostUserId: '',
    createdAfter: '',
    createdBefore: '',
    limit: '100',
    offset: '0',
  });

  const [grantUserId, setGrantUserId] = useState('');
  const [grantAmount, setGrantAmount] = useState('1000000');
  const [grantReason, setGrantReason] = useState('manual_adjustment');
  const [grantError, setGrantError] = useState<string | null>(null);
  const [grantResult, setGrantResult] = useState<GrantResult | null>(null);
  const [appliedFilters, setAppliedFilters] = useState(filters);

  const [grantCredits, { isLoading: granting }] = useGrantCreditsMutation();

  const parsedLimit = parsePositiveInt(appliedFilters.limit);
  const parsedOffset = parsePositiveInt(appliedFilters.offset);
  const parsedGrantAmount = Number.parseInt(grantAmount, 10);

  const {
    data,
    isLoading,
    isError,
    isFetching,
    refetch,
  } = useListAdminUsageQuery({
    userId: appliedFilters.userId || undefined,
    daemonNodeId: appliedFilters.daemonNodeId || undefined,
    hostUserId: appliedFilters.hostUserId || undefined,
    createdAfter: appliedFilters.createdAfter || undefined,
    createdBefore: appliedFilters.createdBefore || undefined,
    limit: parsedLimit,
    offset: parsedOffset,
  });

  const canGrant = grantUserId.trim().length > 0 && Number.isFinite(parsedGrantAmount) && parsedGrantAmount > 0;

  const rows = useMemo(() => data?.data ?? [], [data]);

  return (
    <div className="grid gap-4">
      <DataCard
        title="Manual credit grant"
        action={
          <button
            type="submit"
            form="grant-form"
            disabled={!canGrant || granting}
            className="rounded-lg border border-shell-border px-3 py-2 disabled:opacity-50"
          >
            {granting ? 'Memproses...' : 'Grant'}
          </button>
        }
      >
        <form
          id="grant-form"
          className="grid gap-2 sm:grid-cols-4"
          onSubmit={async (event) => {
            event.preventDefault();
            setGrantError(null);
            try {
              const result = await grantCredits({
                user_id: grantUserId.trim(),
                amount_microcredits: Number.parseInt(grantAmount, 10),
                reason: grantReason,
              }).unwrap();
              setGrantResult({
                user_id: result.user_id,
                balance_microcredits: result.balance_microcredits,
              });
            } catch (error) {
              setGrantResult(null);
              setGrantError(extractApiErrorMessage(error, 'Gagal menambah kredit.'));
            }
          }}
        >
          <input
            value={grantUserId}
            onChange={(event) => setGrantUserId(event.target.value)}
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
            value={grantReason}
            onChange={(event) => setGrantReason(event.target.value)}
            placeholder="Alasan"
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <div className="rounded-lg border border-dashed border-shell-border px-3 py-2 text-xs text-slate-400">
            Selesai: {grantResult ? `${grantResult.user_id} -> ${grantResult.balance_microcredits}` : '-'}
          </div>
        </form>
        {grantError ? <ErrorText>{grantError}</ErrorText> : null}
      </DataCard>

      <DataCard title="Usage filter & list">
        <div className="mb-3 grid gap-2 sm:grid-cols-3">
          <input
            value={filters.userId}
            onChange={(event) => setFilters((prev) => ({ ...prev, userId: event.target.value }))}
            placeholder="Filter user_id"
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <input
            value={filters.daemonNodeId}
            onChange={(event) => setFilters((prev) => ({ ...prev, daemonNodeId: event.target.value }))}
            placeholder="Filter daemon_node_id"
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <input
            value={filters.hostUserId}
            onChange={(event) => setFilters((prev) => ({ ...prev, hostUserId: event.target.value }))}
            placeholder="Filter host_user_id"
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <input
            value={filters.createdAfter}
            type="datetime-local"
            onChange={(event) => setFilters((prev) => ({ ...prev, createdAfter: event.target.value }))}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <input
            value={filters.createdBefore}
            type="datetime-local"
            onChange={(event) => setFilters((prev) => ({ ...prev, createdBefore: event.target.value }))}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          />
          <input
            value={filters.limit}
            onChange={(event) => setFilters((prev) => ({ ...prev, limit: event.target.value }))}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="limit"
          />
          <input
            value={filters.offset}
            onChange={(event) => setFilters((prev) => ({ ...prev, offset: event.target.value }))}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="offset"
          />
          <button
            type="button"
            className="rounded-lg border border-shell-border px-2 py-1 text-xs"
            onClick={() => {
              setAppliedFilters(filters);
              void refetch();
            }}
          >
            Terapkan
          </button>
        </div>

        <QueryState
          isLoading={isLoading || isFetching}
          isError={isError}
          isEmpty={!isLoading && !isFetching && !isError && rows.length === 0}
          isEmptyText="Tidak ada usage dengan filter saat ini."
        />

        {!isLoading && !isError && rows.length > 0 ? (
          <ul className="space-y-2">
            {rows.map((entry) => (
              <li key={entry.id} className="rounded-lg border border-shell-border p-3">
                <p className="font-mono text-sm">{entry.model}</p>
                <p className="text-xs text-slate-400">
                  user={entry.user_id || '-'} · request={entry.request_id.slice(0, 8)} · status=
                  {entry.status_code} · total={entry.usage_total_tokens} token
                </p>
                <p className="text-xs text-slate-500">
                  daemon={entry.daemon_node_id || '-'} · created_at={entry.created_at}
                </p>
                {entry.user_id ? (
                  <button
                    type="button"
                    onClick={() => setGrantUserId(entry.user_id || '')}
                    className="mt-2 rounded-lg border border-shell-border px-2 py-1 text-xs"
                  >
                    Pakai user_id untuk grant
                  </button>
                ) : null}
              </li>
            ))}
          </ul>
        ) : null}
      </DataCard>

      <DataCard title="Catatan">
        <p className="text-xs text-slate-400">
          Tidak ada endpoint user lookup detail saat ini. Gunakan daftar usage di atas untuk menyalin user_id lalu grant
          credits.
        </p>
      </DataCard>
    </div>
  );
}

function parsePositiveInt(raw: string) {
  const parsed = Number.parseInt(raw, 10);
  if (Number.isNaN(parsed) || parsed < 0) {
    return undefined;
  }
  return parsed;
}
