import { DataCard, EmptyState } from '@/components/DataRow';
import { useListUsageQuery } from '@/features/api/mizanApi';

export function UserUsagePage() {
  const { data, isLoading, refetch } = useListUsageQuery({ limit: 20, offset: 0 });

  return (
    <div className="grid gap-4">
      <DataCard title="Riwayat usage">
        <button
          className="mb-3 rounded-lg border border-shell-border px-2 py-1 text-xs"
          type="button"
          onClick={() => refetch()}
        >
          Refresh
        </button>
        {isLoading ? (
          <p>Loading...</p>
        ) : data?.data.length === 0 ? (
          <EmptyState>Belum ada usage.</EmptyState>
        ) : (
          <ul className="space-y-2">
            {data?.data?.map((item) => (
              <li key={item.id} className="rounded-lg border border-shell-border p-3">
                <p className="font-semibold">{item.model}</p>
                <p className="text-sm text-slate-300">
                  prompt {item.usage_prompt_tokens}, completion {item.usage_completion_tokens}, total {item.usage_total_tokens}
                </p>
                <p className="text-xs text-slate-500">{item.created_at}</p>
              </li>
            ))}
          </ul>
        )}
      </DataCard>
    </div>
  );
}
