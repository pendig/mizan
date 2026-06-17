import { useState } from 'react';
import { DataCard, EmptyState } from '@/components/DataRow';
import { useCreateApiKeyMutation, useListApiKeysQuery, useRevokeApiKeyMutation } from '@/features/api/mizanApi';

export function UserKeysPage() {
  const { data, isLoading } = useListApiKeysQuery();
  const [createApiKey, { isLoading: isSaving }] = useCreateApiKeyMutation();
  const [revokeApiKey] = useRevokeApiKeyMutation();

  const [label, setLabel] = useState('');

  return (
    <div className="grid gap-4">
      <DataCard
        title="Buat API key"
        action={
          <button
            type="button"
            onClick={() => createApiKey({ label: label.trim() ? label : undefined })}
            disabled={isSaving}
            className="rounded-lg border border-shell-border px-3 py-2"
          >
            Buat
          </button>
        }
      >
        <input
          value={label}
          onChange={(event) => setLabel(event.target.value)}
          className="w-full rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          placeholder="Label key (opsional)"
        />
      </DataCard>

      <DataCard title="API keys">
        {isLoading ? (
          <p>Loading...</p>
        ) : !data?.keys?.length ? (
          <EmptyState>Belum ada API key.</EmptyState>
        ) : (
          <ul className="space-y-2">
            {data.keys.map((item) => (
              <li key={item.id} className="flex items-center justify-between rounded-lg border border-shell-border p-3">
                <div>
                  <p className="font-medium">{item.label || '(tanpa label)'}</p>
                  <p className="font-mono text-xs text-slate-400">{item.id}</p>
                  <p className="text-xs text-slate-500">dibuat: {item.created_at}</p>
                </div>
                <button
                  type="button"
                  onClick={() => revokeApiKey(item.id)}
                  className="rounded-lg border border-rose-500/60 px-2 py-1 text-sm text-rose-300"
                >
                  Revoke
                </button>
              </li>
            ))}
          </ul>
        )}
      </DataCard>
    </div>
  );
}
