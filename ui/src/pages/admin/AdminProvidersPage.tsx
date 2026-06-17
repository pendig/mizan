import { useState } from 'react';
import { DataCard, EmptyState } from '@/components/DataRow';
import {
  useCreateProviderConnectionMutation,
  useDeleteProviderConnectionMutation,
  useListProviderConnectionsQuery,
} from '@/features/api/mizanApi';

export function AdminProvidersPage() {
  const { data, isLoading } = useListProviderConnectionsQuery();
  const [createProvider, { isLoading: creating }] = useCreateProviderConnectionMutation();
  const [deleteProvider] = useDeleteProviderConnectionMutation();

  const [name, setName] = useState('OpenAI Prod');
  const [providerType, setProviderType] = useState('openai');
  const [baseUrl, setBaseUrl] = useState('https://api.openai.com/v1');

  return (
    <div className="grid gap-4">
      <DataCard
        title="Tambah provider"
        action={
          <button
            type="button"
            onClick={() =>
              createProvider({
                name: name.trim(),
                provider_type: providerType.trim(),
                base_url: baseUrl.trim(),
                enabled: true,
              })
            }
            disabled={creating}
            className="rounded-lg border border-shell-border px-3 py-2"
          >
            Simpan
          </button>
        }
      >
        <div className="grid gap-2 sm:grid-cols-2">
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Nama koneksi"
          />
          <input
            value={providerType}
            onChange={(event) => setProviderType(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Tipe provider"
          />
          <input
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2 sm:col-span-2"
            placeholder="Base URL"
          />
        </div>
      </DataCard>

      <DataCard title="Provider connections">
        {isLoading ? (
          <p>Loading...</p>
        ) : data?.data.length === 0 ? (
          <EmptyState>Belum ada provider.</EmptyState>
        ) : (
          <ul className="space-y-2">
            {data?.data?.map((provider) => (
              <li key={provider.id} className="flex items-center justify-between rounded-lg border border-shell-border p-3">
                <div>
                  <p className="font-medium">{provider.name}</p>
                  <p className="text-xs text-slate-400">
                    {provider.provider_type} · {provider.base_url}
                  </p>
                </div>
                <button
                  onClick={() => deleteProvider(provider.id)}
                  type="button"
                  className="rounded-lg border border-rose-500/60 px-2 py-1 text-sm text-rose-300"
                >
                  Hapus
                </button>
              </li>
            ))}
          </ul>
        )}
      </DataCard>
    </div>
  );
}
