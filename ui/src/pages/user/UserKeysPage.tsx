import { useState } from 'react';
import { DataCard, EmptyState } from '@/components/DataRow';
import { useCreateApiKeyMutation, useListApiKeysQuery, useRevokeApiKeyMutation } from '@/features/api/mizanApi';
import type { ApiKeyCreateResponse } from '@/app/types';

export function UserKeysPage() {
  const { data, isLoading } = useListApiKeysQuery();
  const [createApiKey, { isLoading: isSaving }] = useCreateApiKeyMutation();
  const [revokeApiKey] = useRevokeApiKeyMutation();

  const [label, setLabel] = useState('');
  const [createdKey, setCreatedKey] = useState<ApiKeyCreateResponse | null>(null);

  return (
    <div className="grid gap-4">
      <DataCard
        title="Buat API key"
        action={
          <button
            type="button"
            onClick={async () => {
              setCreatedKey(null);
              const response = await createApiKey({ label: label.trim() ? label.trim() : undefined }).unwrap();
              setCreatedKey(response);
              setLabel('');
            }}
            disabled={isSaving}
            className="rounded-lg border border-shell-border px-3 py-2"
          >
            {isSaving ? 'Membuat...' : 'Buat'}
          </button>
        }
      >
        <input
          value={label}
          onChange={(event) => setLabel(event.target.value)}
          className="w-full rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          placeholder="Label key (opsional)"
        />
        {createdKey ? (
          <div className="mt-3 rounded-lg border border-emerald-400/40 bg-emerald-950/30 p-3 text-xs break-all">
            <p className="mb-1 font-semibold text-emerald-300">API key baru (sekali muncul):</p>
            <p className="font-mono">{createdKey.key}</p>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard.writeText(createdKey.key);
              }}
              className="mt-2 rounded-lg border border-emerald-200/40 px-2 py-1 text-xs"
            >
              Salin key
            </button>
          </div>
        ) : null}
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
